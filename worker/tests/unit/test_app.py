from __future__ import annotations

import asyncio
import io
import os
import time
from pathlib import Path

import aiohttp
import numpy as np
import pytest
from aiohttp import web
from aiohttp.test_utils import TestServer

from tests.conftest import CONFIG_DIR
from tests.fakes.clock import FakeClock
from worker import app, health
from worker import settings as settings_module
from worker.app import (
    EXIT_CRASHED,
    LLM_CALLS_PER_REQUEST,
    Blueprint,
    GpuProbe,
    HttpPools,
    Node,
)
from worker.bus.consumers import ConsumerWatch, LaneState
from worker.contract import Contract
from worker.fetch_models import FASTTEXT_HOME_ENV
from worker.observability.counters import Counters
from worker.observability.health_file import HealthFile
from worker.observability.logging import JsonLog
from worker.runtime.batcher import Batcher
from worker.runtime.engine_client import CAUSE_STOP, EngineKilled, SlotUnavailable
from worker.runtime.protocol import Call, Reply
from worker.runtime.supervisor import STATE_BROKEN, STATE_LOADING, STATE_READY

CALL_DEADLINE_S = 60.0
STOP_LIMIT_S = 2.0


class KillableEngine:
    def __init__(self) -> None:
        self.name = "fake"
        self.pid = os.getpid()
        self.alive = True
        self.started = asyncio.Event()
        self.killed = asyncio.Event()

    async def call(self, call: Call) -> Reply:
        self.started.set()
        await self.killed.wait()
        raise EngineKilled(self.name, CAUSE_STOP)


class OneEnginePool:
    def __init__(self, engine: KillableEngine) -> None:
        self.engine = engine

    async def acquire(self, slot: str, deadline_at: float) -> KillableEngine:
        return self.engine

    def release(self, client: KillableEngine) -> None:
        return None

    def report_oom(self, slot: str, client: KillableEngine) -> None:
        return None


class FakeSupervisor:
    def __init__(self, engine: KillableEngine | None = None) -> None:
        self.engine = engine
        self.states: dict[str, str] = {}

    async def stop(self) -> None:
        if self.engine is not None:
            self.engine.killed.set()

    def slot_state(self, slot: str) -> str:
        return self.states.get(slot, STATE_READY)


@pytest.fixture
def node(base_env: dict[str, str], contract: Contract, tmp_path: Path) -> Node:
    env = {
        **base_env,
        settings_module.PROFILE_ENV: "cpu",
        FASTTEXT_HOME_ENV: str(tmp_path / "fasttext"),
        "WORKER__WORKER__WORK_DIR": str(tmp_path / "work"),
    }
    settings = settings_module.load(CONFIG_DIR, env)
    blueprint = Blueprint.of(settings, contract, env)
    return Node(settings, contract, blueprint, tmp_path / "health.json", JsonLog(io.StringIO()))


async def test_stop_kills_engines_before_waiting_for_their_calls(node: Node) -> None:
    engine = KillableEngine()
    batcher = Batcher("fake", 4, 0, OneEnginePool(engine), Counters(), log=JsonLog(io.StringIO()))
    node.supervisor = FakeSupervisor(engine)
    node.batchers = {"fake": batcher}
    rows = {"x": np.ones((1, 2), np.float32)}
    submit = asyncio.create_task(
        batcher.submit("echo", rows, {}, time.monotonic() + CALL_DEADLINE_S)
    )
    await asyncio.wait_for(engine.started.wait(), STOP_LIMIT_S)

    await asyncio.wait_for(node.stop_runtime(), STOP_LIMIT_S)

    with pytest.raises(SlotUnavailable):
        await asyncio.wait_for(submit, STOP_LIMIT_S)


async def test_crashed_background_task_exits_non_zero(node: Node) -> None:
    async def broken_health() -> None:
        raise OSError("read-only file system: /run/worker")

    task = asyncio.ensure_future(broken_health())
    task.set_name("health")
    task.add_done_callback(node.background_finished)
    await asyncio.gather(task, return_exceptions=True)

    assert await asyncio.wait_for(node.until_stopped(), STOP_LIMIT_S) == EXIT_CRASHED
    assert node.counters.value("background_crashes_total", task="health") == 1


async def test_stop_signal_alone_exits_zero(node: Node) -> None:
    node.stop_requested.set()
    assert await asyncio.wait_for(node.until_stopped(), STOP_LIMIT_S) == 0


def lane_watch(node: Node, contract: Contract, clock: FakeClock) -> ConsumerWatch:
    return ConsumerWatch(
        contract.lane("lyrics"), None, node.connection, node.counters, clock, False, 60.0
    )


async def test_broken_slot_degrades_the_lane_and_loading_only_pauses_it(
    node: Node, contract: Contract, clock: FakeClock, tmp_path: Path
) -> None:
    supervisor = FakeSupervisor()
    node.supervisor = supervisor
    watch = lane_watch(node, contract, clock)
    watch.paused = True

    supervisor.states = {"text": STATE_LOADING}
    node.gate_lane("lyrics", watch)
    assert watch.state is LaneState.PAUSED

    supervisor.states = {"text": STATE_BROKEN}
    node.gate_lane("lyrics", watch)
    assert watch.state is LaneState.DEGRADED

    path = tmp_path / "lanes.json"
    lanes = {"lyrics": {"state": watch.state.value, "since": 0.0}}
    HealthFile(path, wall=lambda: health.DEGRADED_GRACE_S + 1.0).write({"lanes": lanes})
    assert not health.check(path, now=health.DEGRADED_GRACE_S + 2.0).healthy

    supervisor.states = {}
    node.gate_lane("lyrics", watch)
    assert not watch.paused and not watch.engine_broken


async def test_gpu_probe_without_exec_permission_turns_itself_off(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tool = tmp_path / "nvidia-smi"
    tool.write_text("#!/bin/sh\n", encoding="utf-8")
    tool.chmod(0o644)
    monkeypatch.setattr(app, "GPU_QUERY", (str(tool),))
    counters = Counters()
    probe = GpuProbe(counters, enabled=True)

    await asyncio.wait_for(probe.probe(), STOP_LIMIT_S)

    assert probe.enabled is False
    assert counters.value("gpu_probe_failures_total") == 1


class HangingOrigin:
    def __init__(self) -> None:
        self.waiting = 0
        self.release = asyncio.Event()

    async def hang(self, request: web.Request) -> web.Response:
        self.waiting += 1
        await self.release.wait()
        return web.Response(body=b"late")

    async def answer(self, request: web.Request) -> web.Response:
        return web.Response(body=b"{}")

    async def reached(self, count: int) -> None:
        while self.waiting < count:
            await asyncio.sleep(0.01)


async def test_llm_calls_do_not_queue_behind_downloads_or_peak_hedging(node: Node) -> None:
    capacity = node.settings.lanes.capacity
    pools = HttpPools.of(node.blueprint.lanes, capacity)
    origin = HangingOrigin()
    server_app = web.Application()
    server_app.router.add_get("/audio", origin.hang)
    server_app.router.add_post("/llm/hang", origin.hang)
    server_app.router.add_post("/llm/answer", origin.answer)
    server = TestServer(server_app)
    await server.start_server()
    downloads = capacity["audio"] + capacity["transcribe"]
    hedged = LLM_CALLS_PER_REQUEST * capacity["ai"]
    try:
        async with pools.llm_session() as llm, pools.audio_session() as audio:
            downloading = [audio.get(server.make_url("/audio")) for _ in range(downloads)]
            calling = [llm.post(server.make_url("/llm/hang")) for _ in range(hedged)]
            stuck = [asyncio.create_task(request) for request in [*downloading, *calling]]
            await asyncio.wait_for(origin.reached(downloads + hedged), STOP_LIMIT_S)
            timeout = aiohttp.ClientTimeout(total=STOP_LIMIT_S)
            async with llm.post(server.make_url("/llm/answer"), timeout=timeout) as response:
                assert response.status == 200
            origin.release.set()
            for response in await asyncio.gather(*stuck):
                response.release()
    finally:
        await server.close()
