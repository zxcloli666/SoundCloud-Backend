from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from dataclasses import dataclass, field

import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeMsg, FakeNats
from worker.bus.connection import Connection
from worker.bus.consumers import ConsumerWatch
from worker.bus.inflight import Inflight
from worker.bus.lane_runner import LaneRunner, QueueHandler
from worker.bus.lease import Leases
from worker.bus.outbox import Outbox
from worker.contract import Contract, LaneSpec
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Producer
from worker.observability.counters import Counters
from worker.settings import Settings

WORKER_ID = "test-worker"
BUILD = "test-build"
SYNC_VERSION = "s4.test0001.test0002.test0003"

Behaviour = Callable[[Mapping[str, object], Deadline], Awaitable[Outcome]]


class ScriptedProcessor:
    def __init__(self) -> None:
        self.calls: list[Mapping[str, object]] = []
        self.behaviour: Behaviour = self._ok
        self.started = asyncio.Event()
        self.release = asyncio.Event()
        self.release.set()

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        self.calls.append(request)
        self.started.set()
        await self.release.wait()
        return await self.behaviour(request, deadline)

    def hold(self) -> None:
        self.release.clear()

    def resume(self) -> None:
        self.release.set()

    async def _ok(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        return Outcome.ok(mert=[0.0] * 1024, clap=[0.0] * 512, fingerprint=None)


@dataclass
class Harness:
    bus: FakeNats
    clock: FakeClock
    contract: Contract
    lane: LaneSpec
    counters: Counters
    connection: Connection
    outbox: Outbox
    watch: ConsumerWatch
    leases: Leases
    inflight: Inflight
    processor: ScriptedProcessor
    handler: QueueHandler
    runner: LaneRunner
    producer: Producer
    tasks: list[asyncio.Task[None]] = field(default_factory=list)

    def enqueue(self, payload: object, msg_id: str | None = "task:1") -> int:
        headers = {"Nats-Msg-Id": msg_id} if msg_id is not None else None
        return self.bus.enqueue(self.lane.filter_subject, payload, headers)

    async def fetch(self, batch: int = 1) -> list[FakeMsg]:
        subscription = await self.bus.jetstream().pull_subscribe_bind(
            durable=self.lane.durable, stream=self.lane.stream
        )
        return await subscription.fetch(batch=batch, timeout=1)

    def handle(self, msg: FakeMsg, abort: asyncio.Event | None = None) -> asyncio.Task[None]:
        task = asyncio.create_task(self.handler.handle(msg, abort or asyncio.Event()))
        self.tasks.append(task)
        return task

    async def settle(self, turns: int = 20) -> None:
        for _ in range(turns):
            await asyncio.sleep(0)

    def done_published(self) -> list[tuple[str, bytes, dict[str, str]]]:
        return [entry for entry in self.bus.published if entry[0].startswith("done.")]

    def start(self) -> asyncio.Task[None]:
        task = asyncio.create_task(self.runner.run())
        self.tasks.append(task)
        return task

    async def stop(self) -> None:
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        for task in list(self.runner.tasks):
            task.cancel()
        await asyncio.gather(*self.runner.tasks, return_exceptions=True)


async def build_harness(
    fake_nats: FakeNats,
    contract: Contract,
    settings: Settings,
    clock: FakeClock,
    lane_name: str = "audio",
    capacity: int = 2,
    required: bool = False,
) -> Harness:
    lane = contract.lane(lane_name)
    counters = Counters()
    connection = Connection(fake_nats, settings.nats, WORKER_ID, counters, clock)
    await connection.open()
    js = connection.js
    outbox = Outbox(
        js, connection, settings.nats.outbox, counters, clock, WORKER_ID, BUILD, contract.headers
    )
    watch = ConsumerWatch(
        lane, js, connection, counters, clock, required, settings.lanes.required_grace_s
    )
    leases = Leases(
        lane, connection, counters, clock, lambda: watch.max_deliver, contract.headers.msg_id
    )
    inflight = Inflight()
    processor = ScriptedProcessor()
    sync_version = SYNC_VERSION if lane_name == "transcribe" else None
    producer = Producer(WORKER_ID, BUILD, {"mert": "m@1", "clap": "c@1"}, sync_version)
    handler = QueueHandler(
        lane, processor, leases, inflight, outbox, watch, contract, producer, counters, clock
    )
    runner = LaneRunner(lane, watch, handler, js, connection, outbox, capacity, counters, clock)
    return Harness(
        fake_nats,
        clock,
        contract,
        lane,
        counters,
        connection,
        outbox,
        watch,
        leases,
        inflight,
        processor,
        handler,
        runner,
        producer,
    )


@pytest.fixture
async def harness(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> AsyncIterator[Harness]:
    built = await build_harness(fake_nats, contract, settings, clock)
    yield built
    await built.stop()


AUDIO_TASK = {"sc_track_id": "42", "s3_url": "https://s3/x", "upload_generation": 1, "attempt": 1}
ENCODE_TASK = {"model": "lyrics", "text": "hello", "hash": "0" * 64}
