from __future__ import annotations

import asyncio
import dataclasses
import json
import os
import subprocess
import sys
import time
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

import nats
import pytest
from nats.aio.client import Client
from nats.js import JetStreamContext
from nats.js.manager import JetStreamManager

from tests.integration.provision import consumer_config, provision_like_jobs, reset
from worker.bus.connection import Connection
from worker.bus.consumers import EX_CONFIG, ConsumerWatch, LaneState
from worker.bus.inflight import Inflight
from worker.bus.lane_runner import LaneRunner, QueueHandler
from worker.bus.lease import Leases
from worker.bus.outbox import Outbox
from worker.contract import Contract, LaneSpec
from worker.domain.deadline import Deadline, MonotonicClock
from worker.domain.outcome import Outcome, Producer
from worker.observability.counters import Counters
from worker.settings import NatsSection, Settings

pytestmark = pytest.mark.integration

WORKER_ROOT = Path(__file__).resolve().parents[2]
AUDIO_TASK = {"sc_track_id": "42", "s3_url": "https://s3/x", "upload_generation": 1, "attempt": 1}
LYRICS_TASK = {"sc_track_id": "42", "request_id": "lyr:42:1", "text": "la la", "language": None}
LIVE_SYNC_VERSION = "s4.live0001.live0002.live0003"
DRIFT_SCRIPT = """
import asyncio, sys
from pathlib import Path
import nats
from worker import contract as contract_module
from worker.bus.connection import Connection
from worker.bus.consumers import ConfigDrift, ConsumerWatch
from worker.domain.deadline import MonotonicClock
from worker.observability.counters import Counters
from worker.settings import NatsSection, OutboxSettings, PingSettings

async def main() -> int:
    contract = contract_module.load(Path(sys.argv[1]))
    url, user, password = sys.argv[2:5]
    section = NatsSection(url, user, password, PingSettings(5, 2), OutboxSettings(256, 64))
    counters = Counters()
    clock = MonotonicClock()
    connection = Connection(nats.NATS(), section, "drift-check", counters, clock)
    await connection.open()
    lane = contract.lane("audio")
    watch = ConsumerWatch(lane, connection.js, connection, counters, clock, True, 300)
    try:
        await watch.verify_at_start()
    except ConfigDrift as drift:
        print(drift)
        return drift.exit_code
    finally:
        await connection.close()
    return 0

sys.exit(asyncio.run(main()))
"""


def nats_section(url: str, settings: Settings) -> tuple[str, NatsSection]:
    parts = urlsplit(url)
    host = parts.hostname or "127.0.0.1"
    bare = urlunsplit((parts.scheme, f"{host}:{parts.port or 4222}", "", "", ""))
    section = dataclasses.replace(
        settings.nats, url=bare, user=parts.username or "", password=parts.password or ""
    )
    return bare, section


class HoldingProcessor:
    def __init__(self) -> None:
        self.started = asyncio.Event()
        self.release = asyncio.Event()
        self.release.set()
        self.calls = 0

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        self.calls += 1
        self.started.set()
        await self.release.wait()
        return Outcome.ok(mert=[0.0] * 1024, clap=[0.0] * 512, fingerprint=None)


@dataclass
class Live:
    url: str
    admin: Client
    js: JetStreamContext
    jsm: JetStreamManager
    contract: Contract
    settings: Settings

    async def publish_task(self, lane: LaneSpec, payload: Mapping[str, object], msg_id: str) -> int:
        ack = await self.js.publish(
            lane.filter_subject, json.dumps(payload).encode(), headers={"Nats-Msg-Id": msg_id}
        )
        return ack.seq

    async def done_messages(self) -> list[tuple[dict[str, object], dict[str, str]]]:
        info = await self.jsm.stream_info("PIPELINE_DONE")
        messages = []
        for seq in range(info.state.first_seq, info.state.last_seq + 1):
            raw = await self.jsm.get_msg("PIPELINE_DONE", seq)
            messages.append((json.loads(raw.data or b"{}"), dict(raw.headers or {})))
        return messages

    async def messages_left(self, stream: str) -> int:
        return int((await self.jsm.stream_info(stream)).state.messages)


@dataclass
class Worker:
    connection: Connection
    counters: Counters
    watch: ConsumerWatch
    runner: LaneRunner
    outbox: Outbox
    processor: HoldingProcessor
    task: asyncio.Task[None] | None = None

    def start(self) -> None:
        self.task = asyncio.create_task(self.runner.run())

    async def close(self) -> None:
        self.runner.stop_fetching()
        if self.task is not None:
            self.task.cancel()
            await asyncio.gather(self.task, return_exceptions=True)
        await self.runner.cancel()
        await self.runner.unsubscribe()
        await self.connection.close()


async def wait_for(condition: Callable[[], Awaitable[bool]], timeout_s: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if await condition():
            return
        await asyncio.sleep(0.1)
    raise AssertionError("condition not met in time")


@pytest.fixture
async def live(contract: Contract, settings: Settings) -> AsyncIterator[Live]:
    url = os.environ["NATS_TEST_URL"]
    admin = await nats.connect(url)
    jsm = admin.jsm()
    js = admin.jetstream()
    await reset(jsm, contract)
    await provision_like_jobs(jsm, js, contract)
    yield Live(url, admin, js, jsm, contract, settings)
    await reset(jsm, contract)
    await admin.close()


async def build_worker(
    live: Live, lane_name: str, url: str | None = None, capacity: int = 2
) -> Worker:
    _, section = nats_section(url or live.url, live.settings)
    lane = live.contract.lane(lane_name)
    counters = Counters()
    clock = MonotonicClock()
    connection = Connection(nats.NATS(), section, "live-worker", counters, clock)
    await connection.open()
    js = connection.js
    names = live.contract.headers
    outbox = Outbox(js, connection, section.outbox, counters, clock, "live-worker", "live", names)
    watch = ConsumerWatch(lane, js, connection, counters, clock, False, 300)
    leases = Leases(lane, connection, counters, clock, lambda: watch.max_deliver, names.msg_id)
    processor = HoldingProcessor()
    sync_version = LIVE_SYNC_VERSION if lane_name == "transcribe" else None
    producer = Producer("live-worker", "live", {"mert": "m@1", "clap": "c@1"}, sync_version)
    handler = QueueHandler(
        lane, processor, leases, Inflight(), outbox, watch, live.contract, producer, counters, clock
    )
    runner = LaneRunner(lane, watch, handler, js, connection, outbox, capacity, counters, clock)
    return Worker(connection, counters, watch, runner, outbox, processor)


async def test_ok_task_is_published_once_and_acked(live: Live) -> None:
    worker = await build_worker(live, "audio")
    try:
        assert await worker.watch.verify_at_start() is LaneState.SERVING
        assert worker.watch.max_deliver == 5
        worker.start()
        seq = await live.publish_task(
            live.contract.lane("audio"), AUDIO_TASK, "storage-audio:42:1:1"
        )
        await wait_for(lambda: _done(live, 1))
        [(done, headers)] = await live.done_messages()
        assert done["status"] == "ok" and done["sc_track_id"] == "42"
        assert headers["Nats-Msg-Id"] == f"done.audio:index_audio:42:1:1:{seq}:ok"
        assert headers["X-Worker-Id"] == "live-worker" and headers["X-Deliveries"] == "1"
        await wait_for(lambda: _left(live, "INDEX_AUDIO", 0))
        info = await live.jsm.consumer_info("INDEX_AUDIO", "audio-workers")
        assert info.num_ack_pending == 0 and info.num_pending == 0
        assert worker.counters.value("done_total", lane="audio", status="ok") == 1
    finally:
        await worker.close()


async def _done(live: Live, count: int) -> bool:
    return (await live.messages_left("PIPELINE_DONE")) >= count


async def _left(live: Live, stream: str, count: int) -> bool:
    return (await live.messages_left(stream)) == count


async def test_drift_at_start_exits_78(live: Live, contract: Contract) -> None:
    lane = contract.lane("audio")
    await live.jsm.add_consumer("INDEX_AUDIO", consumer_config(lane, ack_wait=61.0))
    bare, section = nats_section(live.url, live.settings)
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            DRIFT_SCRIPT,
            str(WORKER_ROOT / "contract" / "worker-contract.json"),
            bare,
            section.user,
            section.password,
        ],
        capture_output=True,
        text=True,
        check=False,
        cwd=WORKER_ROOT,
        timeout=60,
    )
    assert result.returncode == EX_CONFIG == 78, result.stderr
    assert "ack_wait_s" in result.stdout


async def test_drift_during_work_on_late_delivery_publishes_engine_restarted(
    live: Live, contract: Contract
) -> None:
    lane = contract.lane("audio")
    advisories: list[bytes] = []

    async def on_advisory(msg: object) -> None:
        advisories.append(msg.data)

    await live.admin.subscribe(
        "$JS.EVENT.ADVISORY.CONSUMER.MAX_DELIVERIES.INDEX_AUDIO.audio-workers", cb=on_advisory
    )
    await live.publish_task(lane, AUDIO_TASK, "storage-audio:42:1:1")
    probe = await live.js.pull_subscribe_bind(durable=lane.durable, stream=lane.stream)
    for _ in range(3):
        [msg] = await probe.fetch(batch=1, timeout=5)
        await msg.nak()
    await probe.unsubscribe()
    worker = await build_worker(live, "audio")
    try:
        assert await worker.watch.verify_at_start() is LaneState.SERVING
        worker.processor.release.clear()
        worker.start()
        await asyncio.wait_for(worker.processor.started.wait(), 10)
        await live.jsm.add_consumer("INDEX_AUDIO", consumer_config(lane, ack_wait=61.0))
        assert await worker.watch.check() is LaneState.DRAINING
        assert worker.watch.drifted.is_set()
        assert await worker.runner.drain(grace_s=0.5) == 0
        [(done, headers)] = await live.done_messages()
        assert done["status"] == "failed" and done["reason"] == "engine_restarted"
        assert headers["X-Deliveries"] == "4"
        await wait_for(lambda: _left(live, "INDEX_AUDIO", 0))
        await asyncio.sleep(0.5)
        assert advisories == []
    finally:
        await worker.close()


@pytest.mark.skipif(
    "NATS_TEST_RESTRICTED_URL" not in os.environ, reason="needs NATS_TEST_RESTRICTED_URL"
)
async def test_permission_denial_marks_lane_not_served(live: Live) -> None:
    restricted = os.environ["NATS_TEST_RESTRICTED_URL"]
    denied = await build_worker(live, "transcribe", url=restricted)
    allowed = await build_worker(live, "audio", url=restricted)
    try:
        assert await allowed.watch.verify_at_start() is LaneState.SERVING
        assert await denied.watch.verify_at_start() is LaneState.NOT_SERVED
        assert denied.watch.retry_after_s == 600.0
        assert denied.counters.value("nats_errors_total", kind="permissions") >= 1
        assert await denied.watch.check() is LaneState.NOT_SERVED
    finally:
        await denied.close()
        await allowed.close()


@pytest.mark.skipif("NATS_TEST_CONTAINER" not in os.environ, reason="needs NATS_TEST_CONTAINER")
async def test_server_pause_longer_than_ack_wait_applies_exactly_once(
    live: Live, contract: Contract
) -> None:
    container = os.environ["NATS_TEST_CONTAINER"]
    lane = contract.lane("lyrics")
    worker = await build_worker(live, "lyrics")
    try:
        assert await worker.watch.verify_at_start() is LaneState.SERVING
        worker.processor.release.clear()
        worker.start()
        await live.publish_task(lane, LYRICS_TASK, "embed:42:lyr:42:1")
        await asyncio.wait_for(worker.processor.started.wait(), 10)
        subprocess.run(["podman", "pause", container], check=True)
        try:
            await asyncio.sleep(2 * lane.ack_wait_s)
        finally:
            subprocess.run(["podman", "unpause", container], check=True)
        await asyncio.wait_for(worker.connection.wait_connected(), 30)
        assert worker.connection.reconnects >= 1
        assert worker.counters.value("lease_lost_total", lane="lyrics") == 1
        worker.processor.release.set()
        await wait_for(lambda: _left(live, "EMBED_LYRICS", 0), timeout_s=60)
        await asyncio.sleep(1.0)
        messages = await live.done_messages()
        assert len(messages) == 1
        assert messages[0][0]["status"] == "ok"
        assert worker.processor.calls == 1
        assert worker.counters.value("naks_total", lane="lyrics") == 0
    finally:
        await worker.close()
