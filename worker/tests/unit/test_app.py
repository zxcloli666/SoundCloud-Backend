from __future__ import annotations

import asyncio
import io
import os
import time
from pathlib import Path

import numpy as np
import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from tests.fakes.clock import FakeClock
from worker import app, health
from worker import settings as settings_module
from worker.app import EXIT_CRASHED, Blueprint, GpuProbe, Node, StartupError
from worker.bus.consumers import ConsumerWatch, LaneState
from worker.contract import Contract
from worker.fetch_models import FASTTEXT_HOME_ENV
from worker.observability.counters import Counters
from worker.observability.health_file import HealthFile
from worker.observability.logging import JsonLog
from worker.runtime import cpu_flags
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

    async def acquire(
        self, slot: str, deadline_at: float, *, priority: bool = False
    ) -> KillableEngine:
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

    def slot_state(self, slot: str, *, reserved: bool = False) -> str:
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


def cpu_blueprint(
    base_env: dict[str, str], contract: Contract, tmp_path: Path, **overrides: str
) -> Blueprint:
    env = {
        **base_env,
        settings_module.PROFILE_ENV: "cpu",
        FASTTEXT_HOME_ENV: str(tmp_path / "fasttext"),
        **overrides,
    }
    return Blueprint.of(settings_module.load(CONFIG_DIR, env), contract, env, cpus=8)


def test_cpu_profile_runs_encode_in_its_own_engine_and_lanes_in_parallel_processes(
    base_env: dict[str, str], contract: Contract, tmp_path: Path
) -> None:
    blueprint = cpu_blueprint(base_env, contract, tmp_path)
    plans = {plan.name: plan for plan in blueprint.plans}

    assert {name: plan.slot_names for name, plan in plans.items()} == {
        "audio": ("muq", "mulan"),
        "sync": ("asr", "align", "mms"),
        "sep": ("sep",),
        "train-collab": ("train-collab",),
        "train-taste": ("train-taste",),
        "llm-local": ("llm-local",),
        "encode": ("text", "mulan"),
        "cpu-tools": ("vad", "lid", "fingerprint"),
        "clip": ("clip",),
    }
    assert sorted(name for name, plan in plans.items() if plan.reserved) == ["encode", "llm-local"]
    assert {name: plan.threads for name, plan in plans.items() if plan.threads} == {
        "encode": 4,
        "llm-local": 2,
        "cpu-tools": 1,
    }
    assert [name for name, plan in plans.items() if plan.shares_budget] == ["llm-local"]
    assert {name: dict(plan.env) for name, plan in plans.items() if plan.env} == {
        "llm-local": {"OMP_WAIT_POLICY": "PASSIVE"}
    }
    assert {name: plan.idle_unload_s for name, plan in plans.items() if plan.idle_unload_s} == {
        "llm-local": 600.0
    }
    towers = {
        name: spec.options.get("towers")
        for name, plan in plans.items()
        for spec in plan.slots
        if spec.name == "mulan"
    }
    assert towers == {"audio": "audio", "encode": "text"}
    assert (blueprint.policy.cpu_budget, blueprint.policy.threads) == (7, 0)
    assert blueprint.policy.serial_loads is True
    assert blueprint.plans[0].name == "encode"


def test_a_second_text_replica_takes_lyrics_off_the_encode_engine(
    base_env: dict[str, str], contract: Contract, tmp_path: Path
) -> None:
    blueprint = cpu_blueprint(base_env, contract, tmp_path, WORKER__SLOTS__TEXT__REPLICAS="2")
    hosts = {plan.name: plan.reserved for plan in blueprint.plans if "text" in plan.slot_names}
    assert hosts == {"text": False, "encode": True}
    assert {spec.name for spec in app.split_slots(blueprint.plans)} == {"text", "mulan"}


def test_runtime_threads_from_the_environment_fix_the_bulk_engines(
    base_env: dict[str, str], contract: Contract, tmp_path: Path
) -> None:
    blueprint = cpu_blueprint(base_env, contract, tmp_path, WORKER__RUNTIME__THREADS="6")
    assert blueprint.policy.threads == 6


def test_a_host_without_the_cpu_flags_refuses_to_serve_the_ai_lane(
    base_env: dict[str, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(cpu_flags, "missing_on_this_host", lambda: ["avx2", "fma"])
    settings = settings_module.load(CONFIG_DIR, base_env)

    with pytest.raises(StartupError, match=r"lane ai needs the local model, .* \['avx2', 'fma'\]"):
        app.check_local_llm_cpu(settings)


def test_a_host_without_the_cpu_flags_serves_the_other_lanes(
    base_env: dict[str, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(cpu_flags, "missing_on_this_host", lambda: ["avx2", "fma"])
    env = {**base_env, "WORKER__LANES__ENABLED": '["audio", "lyrics"]'}

    app.check_local_llm_cpu(settings_module.load(CONFIG_DIR, env))


def test_a_supported_host_serves_the_ai_lane(
    base_env: dict[str, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(cpu_flags, "missing_on_this_host", list)

    app.check_local_llm_cpu(settings_module.load(CONFIG_DIR, base_env))


def test_gpu_profiles_keep_encode_on_the_shared_slots(contract: Contract, tmp_path: Path) -> None:
    env = {**BASE_ENV, settings_module.PROFILE_ENV: "gpu-24", FASTTEXT_HOME_ENV: str(tmp_path)}
    blueprint = Blueprint.of(settings_module.load(CONFIG_DIR, env), contract, env, cpus=8)
    assert [plan.name for plan in blueprint.plans if plan.reserved] == ["llm-local"]
    assert blueprint.policy.serial_loads is False
    assert all("towers" not in spec.options for spec in blueprint.specs.values())


def test_the_node_routes_encode_through_priority_batchers(node: Node) -> None:
    assert set(node.priority_batchers) == {"mulan"}
    assert set(node.batchers) >= {"muq", "mulan", "text"}
