from __future__ import annotations

import asyncio
import base64
import json
import logging
from collections.abc import Mapping
from typing import Any, Protocol

import nats.errors
import nats.js.errors
from nats.aio.msg import Msg
from nats.js import JetStreamContext

from worker.bus.connection import Connection, race, sleep_unless
from worker.bus.consumers import ConsumerWatch, LaneState
from worker.bus.inflight import Inflight
from worker.bus.lease import Lease, Leases, Settled, current_lease
from worker.bus.outbox import (
    ConnectionClosed,
    Outbox,
    PublishRejected,
    PublishTimedOut,
    encode,
)
from worker.contract import Contract, ContractError, LaneSpec
from worker.domain.deadline import Clock, Deadline
from worker.domain.outcome import (
    TRANSIENT_FAILURES,
    LeaseDropped,
    Outcome,
    PermanentFailure,
    Producer,
    Reason,
    Status,
    TransientFailure,
)
from worker.observability.counters import Counters

log = logging.getLogger("worker.bus.lane_runner")

FETCH_TIMEOUT_S = 5.0
FETCH_HEARTBEAT_S = 2.0
IDLE_POLL_S = 1.0
FETCH_ERROR_PAUSE_S = 1.0
PUBLISH_GIVE_UP_S = 60.0
DEGRADED_PROBE_S = 60.0
DRAIN_NAK_DELAY_S = 5.0
ABORT_SETTLE_S = 10.0
INVALID_BODY_BYTES = 4096
OPTIONAL_RESULT_FIELDS = ("words",)
ENVELOPE_FIELDS = frozenset({"status", "reason", "detail", "producer"})
DEFS_REF = "#/$defs/"


class Processor(Protocol):
    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome: ...


class Handler(Protocol):
    async def handle(self, msg: Msg, abort: asyncio.Event) -> None: ...


class Aborted(Exception):
    pass


class LaneRunner:
    def __init__(
        self,
        lane: LaneSpec,
        watch: ConsumerWatch,
        handler: Handler,
        js: JetStreamContext,
        connection: Connection,
        outbox: Outbox,
        capacity: int,
        counters: Counters,
        clock: Clock,
    ) -> None:
        self.lane = lane
        self.watch = watch
        self.capacity = capacity
        self.tasks: set[asyncio.Task[None]] = set()
        self._handler = handler
        self._js = js
        self._connection = connection
        self._outbox = outbox
        self._counters = counters
        self._clock = clock
        self._subscription: JetStreamContext.PullSubscription | None = None
        self._stop = asyncio.Event()
        self._abort = asyncio.Event()
        self._freed = asyncio.Event()

    @property
    def inflight(self) -> int:
        return len(self.tasks)

    @property
    def state(self) -> LaneState:
        if self._stop.is_set():
            return LaneState.DRAINING
        state = self.watch.state
        if state is LaneState.SERVING and not self._outbox.has_room:
            return LaneState.PAUSED
        return state

    async def run(self) -> None:
        while not self._stop.is_set():
            self._take_lingering()
            if self.state is not LaneState.SERVING:
                await sleep_unless(self._clock, IDLE_POLL_S, self._stop)
                continue
            if not self._connection.is_connected:
                await race(self._connection.wait_connected(), self._stop.wait())
                continue
            free = self.capacity - self.inflight
            if free <= 0:
                await self._wait_for_room()
                continue
            for msg in await self._fetch(free):
                self._spawn(msg)

    def stop_fetching(self) -> None:
        self._stop.set()

    async def drain(self, grace_s: float) -> int:
        self.stop_fetching()
        await self._wait_tasks(grace_s)
        if self.tasks:
            log.warning(
                "lane_aborting_tasks", extra={"lane": self.lane.name, "tasks": len(self.tasks)}
            )
            self._abort.set()
            await self._wait_tasks(ABORT_SETTLE_S)
        return len(self.tasks)

    async def cancel(self) -> None:
        for task in list(self.tasks):
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)

    async def unsubscribe(self) -> None:
        if self._subscription is None:
            return
        try:
            await self._subscription.unsubscribe()
        except nats.errors.Error as error:
            self._counters.inc("unsubscribe_failures_total", lane=self.lane.name)
            log.warning("unsubscribe_failed", extra={"lane": self.lane.name, "error": str(error)})
        self._subscription = None

    def snapshot(self) -> dict[str, object]:
        return {
            "state": self.state.value,
            "inflight": self.inflight,
            "done": {
                status.value: self._counters.value(
                    "done_total", lane=self.lane.name, status=status.value
                )
                for status in Status
            },
            "naks": self._counters.value("naks_total", lane=self.lane.name),
            "last_error": self.watch.last_error,
        }

    async def _fetch(self, batch: int) -> list[Msg]:
        subscription = await self._bound()
        if subscription is None:
            return []
        try:
            return await subscription.fetch(
                batch=batch, timeout=FETCH_TIMEOUT_S, heartbeat=FETCH_HEARTBEAT_S
            )
        except TimeoutError:
            return []
        except (nats.js.errors.Error, nats.errors.Error, ValueError) as error:
            self._counters.inc("fetch_failures_total", lane=self.lane.name)
            log.warning("fetch_failed", extra={"lane": self.lane.name, "error": str(error)})
            await sleep_unless(self._clock, FETCH_ERROR_PAUSE_S, self._stop)
            return []

    async def _bound(self) -> JetStreamContext.PullSubscription | None:
        if self._subscription is not None:
            return self._subscription
        try:
            self._subscription = await self._js.pull_subscribe_bind(
                durable=self.lane.durable, stream=self.lane.stream
            )
        except (nats.js.errors.Error, nats.errors.Error) as error:
            self._counters.inc("bind_failures_total", lane=self.lane.name)
            log.warning("bind_failed", extra={"lane": self.lane.name, "error": str(error)})
            await sleep_unless(self._clock, FETCH_ERROR_PAUSE_S, self._stop)
            return None
        log.info("lane_bound", extra={"lane": self.lane.name, "durable": self.lane.durable})
        return self._subscription

    async def _wait_for_room(self) -> None:
        self._freed.clear()
        await race(self._clock.sleep(self.lane.heartbeat_s), self._freed.wait(), self._stop.wait())

    def _take_lingering(self) -> None:
        if self._subscription is None:
            return
        pending = self._subscription._sub
        while not pending._pending_queue.empty():
            msg = pending._pending_queue.get_nowait()
            pending._pending_queue.task_done()
            pending._pending_size -= len(msg.data)
            if JetStreamContext.is_status_msg(msg):
                self._counters.inc("lingering_status_total", lane=self.lane.name)
                continue
            self._counters.inc("lingering_taken_total", lane=self.lane.name)
            self._spawn(msg)

    def _spawn(self, msg: Msg) -> None:
        task = asyncio.create_task(self._handler.handle(msg, self._abort))
        self.tasks.add(task)
        task.add_done_callback(self._finished)
        self._counters.gauge("lane_inflight", self.inflight, lane=self.lane.name)

    def _finished(self, task: asyncio.Task[None]) -> None:
        self.tasks.discard(task)
        self._freed.set()
        self._counters.gauge("lane_inflight", self.inflight, lane=self.lane.name)
        if task.cancelled():
            return
        error = task.exception()
        if error is not None:
            self._counters.inc("handler_crashes_total", lane=self.lane.name)
            log.error("handler_crashed", extra={"lane": self.lane.name}, exc_info=error)

    async def _wait_tasks(self, timeout_s: float) -> None:
        deadline = self._clock.now() + timeout_s
        while self.tasks and self._clock.now() < deadline:
            self._freed.clear()
            await race(self._clock.sleep(deadline - self._clock.now()), self._freed.wait())


class QueueHandler:
    def __init__(
        self,
        lane: LaneSpec,
        processor: Processor,
        leases: Leases,
        inflight: Inflight,
        outbox: Outbox,
        watch: ConsumerWatch,
        contract: Contract,
        producer: Producer,
        counters: Counters,
        clock: Clock,
    ) -> None:
        self.lane = lane
        self._processor = processor
        self._leases = leases
        self._inflight = inflight
        self._outbox = outbox
        self._watch = watch
        self._contract = contract
        self._producer = producer
        self._counters = counters
        self._clock = clock
        self._failure_fields = failure_fields(contract, lane, producer)

    async def handle(self, msg: Msg, abort: asyncio.Event) -> None:
        try:
            payload = self._parse(msg)
            correlation = self.lane.correlation_key(payload)
        except (ValueError, ContractError) as error:
            await self._reject_invalid(msg, error)
            return
        if (msg.headers or {}).get(self._contract.headers.msg_id) is None:
            self._counters.inc("task_without_msg_id_total", lane=self.lane.name)
            log.warning(
                "task_without_msg_id",
                extra={"lane": self.lane.name, "correlation": correlation},
            )
        attached = self._leases.attach(msg, payload, correlation)
        if attached.redelivered:
            return
        lease = attached.lease
        log.info("task_started", extra=lease.log_fields())
        heartbeat = asyncio.create_task(lease.heartbeat())
        token = current_lease.set(lease)
        try:
            await self._settle(lease, abort)
        finally:
            current_lease.reset(token)
            heartbeat.cancel()
            await asyncio.gather(heartbeat, return_exceptions=True)
            if not lease.is_settled:
                self._counters.inc("lease_unsettled_total", lane=self.lane.name)
                log.error("lease_left_unsettled", extra=lease.log_fields())
                lease.settle(Settled.DROPPED)

    async def _settle(self, lease: Lease, abort: asyncio.Event) -> None:
        if await self._inflight.join(lease) is Settled.PUBLISHED:
            log.info("task_joined_published", extra=lease.log_fields())
            await lease.ack()
            lease.settle(Settled.PUBLISHED)
            return
        started = self._clock.now()
        outcome = await self._outcome(lease, abort)
        if outcome is None:
            return
        await self._publish_and_ack(lease, outcome, started)

    async def _outcome(self, lease: Lease, abort: asyncio.Event) -> Outcome | None:
        try:
            outcome = await self._process(lease, abort)
        except LeaseDropped:
            log.info("task_dropped", extra=lease.log_fields())
            lease.settle(Settled.DROPPED)
            return None
        except Aborted:
            return await self._abort_outcome(lease)
        except TransientFailure as failure:
            return await self._transient(lease, failure.reason, failure.detail)
        except PermanentFailure as failure:
            return failure.outcome()
        except Exception as error:
            self._counters.inc("process_crashes_total", lane=self.lane.name)
            log.error("process_crashed", extra=lease.log_fields(), exc_info=error)
            detail = f"{type(error).__name__}: {error}"
            return await self._transient(lease, Reason.INTERNAL_ERROR, detail)
        if outcome.reason in TRANSIENT_FAILURES and await self._released_for_retry(lease):
            return None
        return outcome

    async def _process(self, lease: Lease, abort: asyncio.Event) -> Outcome:
        work = asyncio.ensure_future(self._processor.process(lease.payload, lease.deadline))
        stop = asyncio.ensure_future(abort.wait())
        try:
            done, _ = await asyncio.wait({work, stop}, return_when=asyncio.FIRST_COMPLETED)
        except asyncio.CancelledError:
            work.cancel()
            stop.cancel()
            await asyncio.gather(work, stop, return_exceptions=True)
            raise
        stop.cancel()
        await asyncio.gather(stop, return_exceptions=True)
        if work in done:
            return work.result()
        work.cancel()
        await asyncio.gather(work, return_exceptions=True)
        raise Aborted

    async def _abort_outcome(self, lease: Lease) -> Outcome | None:
        max_deliver = self._watch.max_deliver
        if max_deliver <= 0 or lease.num_delivered < max_deliver - 1:
            await lease.release_transient(DRAIN_NAK_DELAY_S)
            return None
        reopenable = Reason.ENGINE_RESTARTED.value in self.lane.reopenable
        reason = Reason.ENGINE_RESTARTED if reopenable else Reason.DEADLINE_EXCEEDED
        return self._failed(reason, "worker shutting down")

    async def _transient(self, lease: Lease, reason: Reason, detail: str | None) -> Outcome | None:
        if await self._released_for_retry(lease):
            return None
        return self._failed(reason, detail)

    async def _released_for_retry(self, lease: Lease) -> bool:
        if lease.is_last_delivery and not lease.is_stale:
            return False
        await lease.release_transient(self.lane.nak_delay(lease.num_delivered))
        return True

    async def _publish_and_ack(self, lease: Lease, outcome: Outcome, started: float) -> None:
        outcome, body = self._encode_within_limit(lease, outcome)
        try:
            await self._publish_result(lease, outcome.status, body)
        except PublishTimedOut:
            await lease.release_transient(self.lane.nak_delay(lease.num_delivered))
            return
        except PublishRejected as rejection:
            outcome = self._failed(Reason.INTERNAL_ERROR, f"publish rejected: {rejection}")
            if not await self._publish_compact(lease, outcome):
                return
        await lease.ack()
        lease.settle(Settled.PUBLISHED)
        self._counters.inc("done_total", lane=self.lane.name, status=outcome.status.value)
        duration_ms = round((self._clock.now() - started) * 1000, 1)
        self._counters.observe("task_ms", duration_ms, lane=self.lane.name)
        log.info(
            "job_finished",
            extra={
                **lease.log_fields(),
                "status": outcome.status.value,
                "reason": outcome.reason.value if outcome.reason else None,
                "duration_ms": duration_ms,
            },
        )

    async def _publish_result(self, lease: Lease, status: Status, body: bytes) -> None:
        while True:
            give_up_after_s = None if lease.is_last_delivery else PUBLISH_GIVE_UP_S
            try:
                await self._publish(lease, status, body, give_up_after_s)
            except PublishTimedOut as timeout:
                if isinstance(timeout, ConnectionClosed) or not lease.is_last_delivery:
                    raise
                self._counters.inc("publish_kept_on_last_delivery_total", lane=self.lane.name)
                log.warning("publish_kept_on_last_delivery", extra=lease.log_fields())
                continue
            return

    async def _publish(
        self, lease: Lease, status: Status, body: bytes, give_up_after_s: float | None
    ) -> None:
        if self.lane.done_subject is None:
            raise ContractError(f"lane {self.lane.name} has no done subject")
        await self._outbox.publish(
            self.lane.done_subject,
            body,
            self._outbox.headers(lease.done_msg_id(status), lease.num_delivered),
            self.lane.name,
            give_up_after_s,
        )

    async def _publish_compact(self, lease: Lease, compact: Outcome) -> bool:
        body = encode(compact.to_done(self.lane.echo_fields(lease.payload), self._producer))
        while True:
            try:
                await self._publish(lease, compact.status, body, None)
            except PublishRejected:
                if not self._watch.publish_blocked:
                    self._watch.publish_blocked = True
                    log.error("lane_publish_blocked", extra=lease.log_fields())
                await self._clock.sleep(DEGRADED_PROBE_S)
                continue
            except PublishTimedOut:
                return False
            if self._watch.publish_blocked:
                self._watch.publish_blocked = False
                log.info("lane_publish_restored", extra=lease.log_fields())
            return True

    def _encode_within_limit(self, lease: Lease, outcome: Outcome) -> tuple[Outcome, bytes]:
        echo = self.lane.echo_fields(lease.payload)
        try:
            body = encode(outcome.to_done(echo, self._producer))
        except ValueError as error:
            outcome = self._failed(Reason.MODEL_OUTPUT_INVALID, f"unencodable result: {error}")
            return outcome, encode(outcome.to_done(echo, self._producer))
        limit = min(self.lane.result_max_bytes, self._contract.max_message_bytes)
        if len(body) <= limit:
            return outcome, body
        trimmed = dict(outcome.fields)
        for field in OPTIONAL_RESULT_FIELDS:
            trimmed.pop(field, None)
        if len(trimmed) != len(outcome.fields):
            outcome = Outcome(outcome.status, outcome.reason, outcome.detail, trimmed)
            body = encode(outcome.to_done(echo, self._producer))
            if len(body) <= limit:
                self._counters.inc("result_trimmed_total", lane=self.lane.name)
                return outcome, body
        self._counters.inc("result_too_large_total", lane=self.lane.name)
        log.warning("result_too_large", extra={**lease.log_fields(), "bytes": len(body)})
        outcome = self._failed(Reason.MODEL_OUTPUT_INVALID, f"result_bytes={len(body)} > {limit}")
        return outcome, encode(outcome.to_done(echo, self._producer))

    def _failed(self, reason: Reason, detail: str | None) -> Outcome:
        return Outcome.failed(reason, detail, **self._failure_fields)

    def _parse(self, msg: Msg) -> Mapping[str, object]:
        payload = json.loads(msg.data, parse_constant=reject_constant)
        if not isinstance(payload, dict):
            raise ValueError("payload is not a JSON object")
        return payload

    async def _reject_invalid(self, msg: Msg, error: Exception) -> None:
        stream_seq = msg.metadata.sequence.stream
        msg_id = (msg.headers or {}).get(self._contract.headers.msg_id)
        self._counters.inc("invalid_tasks_total", lane=self.lane.name)
        log.warning(
            "task_invalid",
            extra={
                "lane": self.lane.name,
                "stream_seq": stream_seq,
                "nats_msg_id": msg_id,
                "error": str(error)[:256],
            },
        )
        event = {
            "lane": self.lane.name,
            "stream": self.lane.stream,
            "stream_seq": stream_seq,
            "nats_msg_id": msg_id,
            "error": str(error)[:256],
            "body_base64": base64.b64encode(msg.data[:INVALID_BODY_BYTES]).decode(),
        }
        try:
            await self._outbox.publish(
                self._contract.invalid_subject(self.lane.name),
                encode(event),
                self._outbox.headers(
                    f"invalid:{self.lane.stream}:{stream_seq}", msg.metadata.num_delivered
                ),
                self.lane.name,
                PUBLISH_GIVE_UP_S,
            )
        except (PublishRejected, PublishTimedOut) as failure:
            self._counters.inc("invalid_event_failures_total", lane=self.lane.name)
            log.error(
                "invalid_event_lost",
                extra={"lane": self.lane.name, "stream_seq": stream_seq, "error": str(failure)},
            )
        try:
            await msg.term()
        except nats.errors.Error as failure:
            self._counters.inc("term_failures_total", lane=self.lane.name)
            log.error(
                "term_failed",
                extra={"lane": self.lane.name, "stream_seq": stream_seq, "error": str(failure)},
            )


def reject_constant(constant: str) -> float:
    raise ValueError(f"non-finite number {constant} in payload")


def failure_fields(contract: Contract, lane: LaneSpec, producer: Producer) -> dict[str, object]:
    if lane.done_subject is None:
        return {}
    schema: Mapping[str, Any] = contract.schemas[lane.done_subject]
    properties: Mapping[str, Any] = schema.get("properties", {})
    defs: Mapping[str, Any] = schema.get("$defs", {})
    wire = producer.to_wire()
    skipped = ENVELOPE_FIELDS | set(lane.echo.values())
    return {
        name: failure_value(lane, name, properties[name], defs, wire)
        for name in schema.get("required", ())
        if name not in skipped
    }


def failure_value(
    lane: LaneSpec,
    name: str,
    schema: Mapping[str, Any],
    defs: Mapping[str, Any],
    producer: Mapping[str, object],
) -> object:
    if producer.get(name) is not None:
        return producer[name]
    resolved = resolve(schema, defs)
    if "const" in resolved:
        return resolved["const"]
    if accepts_null(resolved, defs):
        return None
    if resolved.get("type") == "boolean":
        return False
    if resolved.get("type") == "integer" and "minimum" in resolved:
        return resolved["minimum"]
    raise ContractError(f"lane {lane.name}: no failure value for required field {name!r}")


def resolve(schema: Mapping[str, Any], defs: Mapping[str, Any]) -> Mapping[str, Any]:
    ref = schema.get("$ref")
    if isinstance(ref, str) and ref.startswith(DEFS_REF):
        return resolve(defs[ref.removeprefix(DEFS_REF)], defs)
    return schema


def accepts_null(schema: Mapping[str, Any], defs: Mapping[str, Any]) -> bool:
    kind = schema.get("type")
    if kind == "null" or (isinstance(kind, list) and "null" in kind):
        return True
    options = [*schema.get("oneOf", ()), *schema.get("anyOf", ())]
    return any(accepts_null(resolve(option, defs), defs) for option in options)
