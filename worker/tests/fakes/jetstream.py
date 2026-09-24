from __future__ import annotations

import asyncio
import datetime
import json
from collections import deque
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, field

import nats.errors
import nats.js.errors
from nats.aio.msg import Msg
from nats.js import api

from tests.fakes.clock import FakeClock
from tests.fakes.object_store import FakeObjectStore

PERMISSIONS_VIOLATION = "nats: Permissions Violation for Publish to"


class ForbiddenCall(AssertionError):
    pass


@dataclass
class StoredMessage:
    seq: int
    subject: str
    data: bytes
    headers: dict[str, str]
    stored_at: float


@dataclass
class Pending:
    seq: int
    num_delivered: int
    available_at: float
    delivered: bool


@dataclass
class SentAck:
    kind: str
    stream: str
    consumer: str
    seq: int
    num_delivered: int
    delay: float | None
    at: float


@dataclass
class FakeStream:
    name: str
    subjects: tuple[str, ...]
    retention: str = "work_queue"
    max_age_s: float = 86_400.0
    duplicate_window_s: float = 120.0
    max_bytes: int = 1 << 30
    messages: dict[int, StoredMessage] = field(default_factory=dict)
    msg_ids: dict[str, tuple[int, float]] = field(default_factory=dict)
    last_seq: int = 0

    def store(
        self, subject: str, data: bytes, headers: Mapping[str, str], now: float
    ) -> api.PubAck:
        msg_id = headers.get(api.Header.MSG_ID)
        if msg_id is not None:
            known = self.msg_ids.get(msg_id)
            if known is not None and now - known[1] <= self.duplicate_window_s:
                return api.PubAck(stream=self.name, seq=known[0], duplicate=True)
        self.last_seq += 1
        self.messages[self.last_seq] = StoredMessage(
            self.last_seq, subject, data, dict(headers), now
        )
        if msg_id is not None:
            self.msg_ids[msg_id] = (self.last_seq, now)
        return api.PubAck(stream=self.name, seq=self.last_seq, duplicate=False)

    def serves(self, subject: str) -> bool:
        return any(subject_matches(pattern, subject) for pattern in self.subjects)

    @property
    def bytes(self) -> int:
        return sum(len(message.data) for message in self.messages.values())


@dataclass
class FakeConsumer:
    stream: FakeStream
    config: api.ConsumerConfig
    pending: dict[int, Pending] = field(default_factory=dict)
    exhausted: set[int] = field(default_factory=set)
    advisories: list[dict[str, object]] = field(default_factory=list)
    consumer_seq: int = 0

    @property
    def durable(self) -> str:
        return str(self.config.durable_name)

    @property
    def ack_wait(self) -> float:
        return float(self.config.ack_wait or 30.0)

    @property
    def max_deliver(self) -> int:
        return int(self.config.max_deliver or -1)

    def due(self, now: float) -> list[int]:
        due: list[int] = []
        for seq, message in sorted(self.stream.messages.items()):
            if seq in self.exhausted or not subject_matches(
                str(self.config.filter_subject or ">"), message.subject
            ):
                continue
            pending = self.pending.get(seq)
            if pending is None:
                due.append(seq)
            elif pending.available_at <= now:
                if self.max_deliver > 0 and pending.num_delivered >= self.max_deliver:
                    self.exhausted.add(seq)
                    self.advisories.append(
                        {
                            "type": "io.nats.jetstream.advisory.v1.max_deliver",
                            "stream": self.stream.name,
                            "consumer": self.durable,
                            "stream_seq": seq,
                            "deliveries": pending.num_delivered,
                        }
                    )
                    continue
                due.append(seq)
        return due

    def deliver(self, seq: int, now: float) -> Pending:
        pending = self.pending.get(seq)
        if pending is None:
            pending = Pending(seq, 0, now, False)
            self.pending[seq] = pending
        pending.num_delivered += 1
        pending.available_at = now + self.ack_wait
        pending.delivered = True
        self.consumer_seq += 1
        return pending

    def progress(self, seq: int, now: float) -> None:
        pending = self.pending.get(seq)
        if pending is not None:
            pending.available_at = now + self.ack_wait

    def nak(self, seq: int, delay: float | None, now: float) -> None:
        pending = self.pending.get(seq)
        if pending is not None:
            pending.available_at = now + (delay or 0.0)

    def settle(self, seq: int) -> None:
        self.pending.pop(seq, None)
        self.exhausted.discard(seq)
        if self.stream.retention == "work_queue":
            self.stream.messages.pop(seq, None)

    def num_pending(self, now: float) -> int:
        return len(self.due(now))


class FakeMsg:
    def __init__(
        self,
        bus: FakeNats,
        consumer: FakeConsumer,
        stored: StoredMessage,
        pending: Pending,
        now: float,
    ) -> None:
        self._bus = bus
        self._consumer = consumer
        self._ackd = False
        self.subject = stored.subject
        self.data = stored.data
        self.headers: dict[str, str] | None = dict(stored.headers) or None
        timestamp_ns = int(now * 1_000_000_000)
        self.reply = (
            f"$JS.ACK.{consumer.stream.name}.{consumer.durable}.{pending.num_delivered}."
            f"{stored.seq}.{consumer.consumer_seq}.{timestamp_ns}.{consumer.num_pending(now)}"
        )

    @property
    def header(self) -> dict[str, str] | None:
        return self.headers

    @property
    def metadata(self) -> Msg.Metadata:
        return Msg.Metadata._from_reply(self.reply)

    @property
    def seq(self) -> int:
        return self.metadata.sequence.stream

    async def ack(self) -> None:
        self._check_reply()
        await self._bus.send_ack(self, "ack", None)
        self._ackd = True

    async def ack_sync(self, timeout: float = 1.0) -> FakeMsg:
        self._check_reply()
        await self._bus.send_ack_sync(self, timeout)
        self._ackd = True
        return self

    async def nak(self, delay: float | None = None) -> None:
        self._check_reply()
        await self._bus.send_ack(self, "nak", delay)
        self._ackd = True

    async def in_progress(self) -> None:
        await self._bus.send_ack(self, "in_progress", None)

    async def term(self) -> None:
        self._check_reply()
        await self._bus.send_ack(self, "term", None)
        self._ackd = True

    def _check_reply(self) -> None:
        if self._ackd:
            raise nats.errors.MsgAlreadyAckdError(self)


class FakeCoreSubscription:
    def __init__(
        self, bus: FakeNats, subject: str, callback: Callable[[object], Awaitable[None]] | None
    ) -> None:
        self._bus = bus
        self.subject = subject
        self.callback = callback
        self.active = True

    async def unsubscribe(self, limit: int = 0) -> None:
        self.active = False

    async def drain(self) -> None:
        self.active = False


class FakePullSubscription:
    def __init__(self, bus: FakeNats, consumer: FakeConsumer) -> None:
        self._bus = bus
        self._consumer = consumer
        self._sub = FakePending()
        self.fetches: list[tuple[int, float | None, float | None]] = []
        self.active = True

    async def fetch(
        self, batch: int = 1, timeout: float | None = 5, heartbeat: float | None = None
    ) -> list[FakeMsg]:
        if not self.active:
            raise ValueError("nats: invalid subscription")
        if batch < 1:
            raise ValueError("nats: invalid batch size")
        self.fetches.append((batch, timeout, heartbeat))
        messages = self._drain_lingering(batch)
        if messages and batch > 1 and heartbeat:
            raise nats.js.errors.APIError(code=400, description="heartbeat value too large")
        if messages:
            return messages
        await asyncio.sleep(0)
        if not self._bus.is_connected:
            raise nats.errors.TimeoutError
        messages = self._bus.deliver_due(self._consumer, batch)
        if messages:
            return messages
        raise nats.js.errors.FetchTimeoutError

    async def unsubscribe(self) -> None:
        self.active = False

    async def consumer_info(self) -> api.ConsumerInfo:
        return await self._bus.consumer_info(self._consumer.stream.name, self._consumer.durable)

    @property
    def pending_msgs(self) -> int:
        return self._sub._pending_queue.qsize()

    def _drain_lingering(self, batch: int) -> list[FakeMsg]:
        queue = self._sub._pending_queue
        messages: list[FakeMsg] = []
        while not queue.empty() and (batch > 1 or not messages):
            message = queue.get_nowait()
            self._sub._pending_size -= len(message.data)
            if isinstance(message, FakeMsg):
                messages.append(message)
        return messages


class FakePending:
    def __init__(self) -> None:
        self._pending_queue: asyncio.Queue[FakeMsg | Msg] = asyncio.Queue()
        self._pending_size = 0

    def put(self, message: FakeMsg | Msg) -> None:
        self._pending_queue.put_nowait(message)
        self._pending_size += len(message.data)


class FakeJetStream:
    def __init__(self, bus: FakeNats) -> None:
        self._bus = bus

    async def publish(
        self,
        subject: str,
        payload: bytes = b"",
        timeout: float | None = None,
        stream: str | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> api.PubAck:
        return await self._bus.js_publish(subject, payload, headers or {}, timeout or 5.0)

    async def pull_subscribe_bind(
        self,
        durable: str | None = None,
        stream: str | None = None,
        inbox_prefix: bytes = api.INBOX_PREFIX,
        pending_msgs_limit: int = 0,
        pending_bytes_limit: int = 0,
        consumer: str | None = None,
        name: str | None = None,
    ) -> FakePullSubscription:
        target = durable or consumer or name
        found = self._bus.consumers.get((str(stream), str(target)))
        if found is None:
            raise nats.js.errors.NotFoundError(code=404, description="consumer not found")
        subscription = FakePullSubscription(self._bus, found)
        self._bus.bound.append(subscription)
        return subscription

    async def pull_subscribe(self, *args: object, **kwargs: object) -> FakePullSubscription:
        raise ForbiddenCall("pull_subscribe creates consumers; use pull_subscribe_bind (I5)")

    async def add_consumer(self, *args: object, **kwargs: object) -> api.ConsumerInfo:
        raise ForbiddenCall("the worker never creates consumers (I5)")

    async def consumer_info(
        self, stream: str, consumer: str, timeout: float | None = None
    ) -> api.ConsumerInfo:
        return await self._bus.consumer_info(stream, consumer, timeout)

    async def account_info(self) -> api.AccountInfo:
        if self._bus.api_faults:
            raise self._bus.api_faults.popleft()
        await asyncio.sleep(0)
        if not self._bus.is_connected:
            raise nats.errors.TimeoutError
        self._bus.account_infos += 1
        return api.AccountInfo.from_response(
            {"memory": 0, "storage": 0, "streams": len(self._bus.streams), "consumers": 0}
        )

    async def stream_info(self, name: str, subjects_filter: str | None = None) -> api.StreamInfo:
        found = self._bus.streams.get(name)
        if found is None:
            raise nats.js.errors.NotFoundError(code=404, description="stream not found")
        config = api.StreamConfig(name=name, subjects=list(found.subjects))
        state = api.StreamState(
            messages=len(found.messages),
            bytes=found.bytes,
            first_seq=min(found.messages, default=0),
            last_seq=found.last_seq,
            consumer_count=sum(1 for key in self._bus.consumers if key[0] == name),
        )
        return api.StreamInfo(config=config, state=state)

    async def object_store(self, bucket: str) -> FakeObjectStore:
        store = self._bus.object_stores.get(bucket)
        if store is None:
            raise nats.js.errors.BucketNotFoundError(code=404, description="bucket not found")
        return store


class FakeNats:
    def __init__(self, clock: FakeClock | None = None) -> None:
        self.clock = clock or FakeClock()
        self.streams: dict[str, FakeStream] = {}
        self.consumers: dict[tuple[str, str], FakeConsumer] = {}
        self.object_stores: dict[str, FakeObjectStore] = {}
        self.consumer_faults: dict[str, object] = {}
        self.publish_faults: deque[Exception] = deque()
        self.api_faults: deque[Exception] = deque()
        self.account_infos = 0
        self.lost_pubacks = 0
        self.max_payload = 1 << 20
        self.published: list[tuple[str, bytes, dict[str, str]]] = []
        self.core_published: list[tuple[str, bytes, dict[str, str] | None]] = []
        self.responders: dict[str, Callable[[bytes], bytes]] = {}
        self.sent: list[SentAck] = []
        self.bound: list[FakePullSubscription] = []
        self.subscriptions: list[FakeCoreSubscription] = []
        self.errors: list[Exception] = []
        self.is_connected = False
        self.closed = False
        self.flushes = 0
        self.reconnects = 0
        self.connect_options: dict[str, object] = {}
        self._buffered: deque[Callable[[], None]] = deque()
        self._js = FakeJetStream(self)

    async def connect(self, **options: object) -> FakeNats:
        self.connect_options = dict(options)
        self.is_connected = True
        return self

    def jetstream(self, **options: object) -> FakeJetStream:
        return self._js

    def jsm(self, **options: object) -> FakeJetStream:
        return self._js

    def provision_stream(
        self,
        name: str,
        subjects: tuple[str, ...],
        retention: str = "work_queue",
        max_age_s: float = 86_400.0,
        duplicate_window_s: float = 120.0,
    ) -> FakeStream:
        stream = FakeStream(name, subjects, retention, max_age_s, duplicate_window_s)
        self.streams[name] = stream
        return stream

    def provision_consumer(self, stream: str, config: api.ConsumerConfig) -> FakeConsumer:
        consumer = FakeConsumer(self.streams[stream], config)
        self.consumers[(stream, consumer.durable)] = consumer
        return consumer

    def provision_object_store(self, bucket: str) -> FakeObjectStore:
        store = FakeObjectStore(bucket)
        self.object_stores[bucket] = store
        return store

    def enqueue(
        self, subject: str, payload: object, headers: Mapping[str, str] | None = None
    ) -> int:
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        stream = self.stream_for(subject)
        return stream.store(subject, data, headers or {}, self.clock.now()).seq

    def stream_for(self, subject: str) -> FakeStream:
        for stream in self.streams.values():
            if stream.serves(subject):
                return stream
        raise nats.js.errors.NoStreamResponseError

    def linger(self, subscription: FakePullSubscription, seq: int) -> FakeMsg:
        consumer = subscription._consumer
        message = self._deliver(consumer, seq)
        subscription._sub.put(message)
        return message

    def linger_status(self, subscription: FakePullSubscription, status: str) -> Msg:
        message = Msg(self, subject=subscription._consumer.durable, headers={"Status": status})
        subscription._sub.put(message)
        return message

    def deliver_due(self, consumer: FakeConsumer, batch: int) -> list[FakeMsg]:
        now = self.clock.now()
        return [self._deliver(consumer, seq) for seq in consumer.due(now)[:batch]]

    def _deliver(self, consumer: FakeConsumer, seq: int) -> FakeMsg:
        now = self.clock.now()
        pending = consumer.deliver(seq, now)
        return FakeMsg(self, consumer, consumer.stream.messages[seq], pending, now)

    async def disconnect(self) -> None:
        self.is_connected = False
        callback = self.connect_options.get("disconnected_cb")
        if callback is not None:
            await callback()

    async def reconnect(self) -> None:
        self.is_connected = True
        self.reconnects += 1
        while self._buffered:
            self._buffered.popleft()()
        callback = self.connect_options.get("reconnected_cb")
        if callback is not None:
            await callback()

    async def report_error(self, error: Exception) -> None:
        self.errors.append(error)
        callback = self.connect_options.get("error_cb")
        if callback is not None:
            await callback(error)

    async def send_ack(self, message: FakeMsg, kind: str, delay: float | None) -> None:
        if self.closed:
            raise nats.errors.ConnectionClosedError
        apply = self._ack_applier(message, kind, delay)
        if self.is_connected:
            apply()
        else:
            self._buffered.append(apply)

    async def send_ack_sync(self, message: FakeMsg, timeout: float) -> None:
        if self.closed:
            raise nats.errors.ConnectionClosedError
        await asyncio.sleep(0)
        if not self.is_connected:
            raise nats.errors.TimeoutError
        self._ack_applier(message, "ack_sync", None)()

    def _ack_applier(self, message: FakeMsg, kind: str, delay: float | None) -> Callable[[], None]:
        consumer = message._consumer
        seq = message.seq
        num_delivered = message.metadata.num_delivered

        def apply() -> None:
            now = self.clock.now()
            self.sent.append(
                SentAck(
                    kind, consumer.stream.name, consumer.durable, seq, num_delivered, delay, now
                )
            )
            if kind in ("ack", "ack_sync", "term"):
                consumer.settle(seq)
            elif kind == "nak":
                consumer.nak(seq, delay, now)
            elif kind == "in_progress":
                consumer.progress(seq, now)

        return apply

    async def js_publish(
        self, subject: str, payload: bytes, headers: Mapping[str, str], timeout: float
    ) -> api.PubAck:
        if len(payload) > self.max_payload:
            raise nats.errors.MaxPayloadError
        if self.publish_faults:
            raise self.publish_faults.popleft()
        await asyncio.sleep(0)
        if not self.is_connected:
            raise nats.errors.TimeoutError
        stream = self.stream_for(subject)
        ack = stream.store(subject, payload, headers, self.clock.now())
        self.published.append((subject, payload, dict(headers)))
        if self.lost_pubacks > 0:
            self.lost_pubacks -= 1
            raise nats.errors.TimeoutError
        return ack

    async def consumer_info(
        self, stream: str, durable: str, timeout: float | None = None
    ) -> api.ConsumerInfo:
        fault = self.consumer_faults.get(durable)
        if fault == "permissions":
            await self.report_error(
                nats.errors.Error(
                    f'{PERMISSIONS_VIOLATION} "$JS.API.CONSUMER.INFO.{stream}.{durable}"'
                )
            )
            raise nats.errors.TimeoutError
        if isinstance(fault, Exception):
            raise fault
        await asyncio.sleep(0)
        if not self.is_connected:
            raise nats.errors.TimeoutError
        consumer = self.consumers.get((stream, durable))
        if consumer is None:
            raise nats.js.errors.NotFoundError(code=404, description="consumer not found")
        now = self.clock.now()
        return api.ConsumerInfo(
            name=durable,
            stream_name=stream,
            config=consumer.config,
            created=datetime.datetime.fromtimestamp(0, datetime.UTC),
            num_ack_pending=sum(1 for p in consumer.pending.values() if p.available_at > now),
            num_pending=consumer.num_pending(now),
            num_waiting=0,
        )

    async def publish(
        self,
        subject: str,
        payload: bytes = b"",
        reply: str = "",
        headers: Mapping[str, str] | None = None,
    ) -> None:
        if self.closed:
            raise nats.errors.ConnectionClosedError
        record = (subject, payload, dict(headers) if headers else None)
        if self.is_connected:
            self.core_published.append(record)
        else:
            self._buffered.append(lambda: self.core_published.append(record))

    async def subscribe(
        self,
        subject: str,
        queue: str = "",
        cb: Callable[[object], Awaitable[None]] | None = None,
        **options: object,
    ) -> FakeCoreSubscription:
        subscription = FakeCoreSubscription(self, subject, cb)
        self.subscriptions.append(subscription)
        return subscription

    async def deliver_core(
        self, subject: str, data: bytes, headers: Mapping[str, str] | None = None
    ) -> int:
        delivered = 0
        for subscription in self.subscriptions:
            if subscription.active and subject_matches(subscription.subject, subject):
                message = Msg(
                    self, subject=subject, data=data, headers=dict(headers) if headers else None
                )
                if subscription.callback is not None:
                    await subscription.callback(message)
                delivered += 1
        return delivered

    async def request(
        self,
        subject: str,
        payload: bytes = b"",
        timeout: float = 0.5,
        headers: Mapping[str, str] | None = None,
    ) -> Msg:
        await asyncio.sleep(0)
        if not self.is_connected:
            raise nats.errors.TimeoutError
        responder = self.responders.get(subject)
        if responder is None:
            raise nats.errors.NoRespondersError
        return Msg(self, subject=subject, data=responder(payload))

    async def flush(self, timeout: float = 10) -> None:
        if not self.is_connected:
            raise nats.errors.FlushTimeoutError
        self.flushes += 1

    async def drain(self) -> None:
        await self.close()

    async def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        self.is_connected = False
        for name in ("disconnected_cb", "closed_cb"):
            callback = self.connect_options.get(name)
            if callback is not None:
                await callback()

    def acks(self, kind: str | None = None) -> list[SentAck]:
        return [sent for sent in self.sent if kind is None or sent.kind == kind]


def subject_matches(pattern: str, subject: str) -> bool:
    pattern_tokens = pattern.split(".")
    subject_tokens = subject.split(".")
    for index, token in enumerate(pattern_tokens):
        if token == ">":
            return len(subject_tokens) > index
        if index >= len(subject_tokens):
            return False
        if token != "*" and token != subject_tokens[index]:
            return False
    return len(pattern_tokens) == len(subject_tokens)
