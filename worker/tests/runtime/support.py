from __future__ import annotations

import asyncio
import io
import os
import time
from collections.abc import Callable, Iterable
from pathlib import Path

from worker.observability.counters import Counters
from worker.observability.logging import JsonLog
from worker.runtime.clock import MonotonicClock
from worker.runtime.engine_client import ENGINE_MODULE, EngineClient, Launch
from worker.runtime.protocol import SlotSpec
from worker.runtime.supervisor import (
    STATE_BROKEN,
    STATE_READY,
    STATE_STOPPED,
    STATE_UNLOADED,
    EnginePlan,
    RuntimePolicy,
    Supervisor,
)

WORKER_ROOT = Path(__file__).resolve().parents[2]
FAKE_LOADER = "tests.runtime.fake_slots:Fake"
LOAD_TIMEOUT_S = 30.0
SETTLED_STATES = frozenset({STATE_READY, STATE_UNLOADED, STATE_BROKEN, STATE_STOPPED})

TEST_POLICY = RuntimePolicy(
    threads=1,
    ping_interval_s=0.2,
    ping_timeout_s=1.0,
    load_timeout_s=LOAD_TIMEOUT_S,
    command_timeout_s=5.0,
    respawn_backoff_s=(0.05, 0.1, 0.2),
    breaker_deaths=3,
    breaker_window_s=30.0,
    breaker_open_s=0.6,
    kill_join_s=2.0,
    stop_grace_s=2.0,
)


def launch(**env: str) -> Launch:
    return Launch(env={"PYTHONPATH": str(WORKER_ROOT), **env})


def fake_spec(
    name: str = "fake",
    max_batch: int = 8,
    max_wait_ms: int = 10,
    device: str = "none",
    **options: object,
) -> SlotSpec:
    return SlotSpec(
        name=name,
        loader=FAKE_LOADER,
        model="fake",
        revision="",
        device=device,
        max_batch=max_batch,
        max_wait_ms=max_wait_ms,
        options=options,
    )


def quiet_log() -> JsonLog:
    return JsonLog(stream=io.StringIO())


async def spawn_client(
    spec: SlotSpec, counters: Counters | None = None, name: str = "engine"
) -> EngineClient:
    client = EngineClient(
        name, (spec,), launch(), counters or Counters(), MonotonicClock(), quiet_log()
    )
    await client.spawn()
    await client.load(spec.name, LOAD_TIMEOUT_S)
    return client


async def started_supervisor(
    plans: list[EnginePlan],
    policy: RuntimePolicy = TEST_POLICY,
    counters: Counters | None = None,
) -> Supervisor:
    supervisor = Supervisor(plans, policy, counters or Counters(), launch=launch(), log=quiet_log())
    await supervisor.start()
    await settled(supervisor, supervisor.slots)
    return supervisor


async def settled(supervisor: Supervisor, slots: Iterable[str]) -> dict[str, str]:
    wanted = tuple(slots)
    await wait_until(
        lambda: all(supervisor.slot_state(slot) in SETTLED_STATES for slot in wanted),
        LOAD_TIMEOUT_S,
    )
    return {slot: supervisor.slot_state(slot) for slot in wanted}


async def wait_until(predicate: Callable[[], bool], timeout_s: float, step_s: float = 0.02) -> bool:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if predicate():
            return True
        await asyncio.sleep(step_s)
    return predicate()


def process_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


async def process_gone(pid: int, timeout_s: float = 5.0) -> bool:
    return await wait_until(lambda: not process_alive(pid) or is_zombie(pid), timeout_s)


def engine_children() -> list[int]:
    children = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="ascii")
            cmdline = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        fields = stat.rsplit(")", 1)[1].split()
        if fields[0] != "Z" and int(fields[1]) == os.getpid() and ENGINE_MODULE.encode() in cmdline:
            children.append(int(entry.name))
    return children


def is_zombie(pid: int) -> bool:
    try:
        status = Path(f"/proc/{pid}/status").read_text(encoding="ascii")
    except OSError:
        return True
    return "State:\tZ" in status
