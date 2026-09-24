from __future__ import annotations

import asyncio
import os
import signal
import time
from dataclasses import replace
from pathlib import Path

import numpy as np
import pytest

from tests.runtime.support import (
    TEST_POLICY,
    engine_children,
    fake_spec,
    launch,
    process_alive,
    process_gone,
    quiet_log,
    settled,
    started_supervisor,
    wait_until,
)
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    DeadlineExceeded,
    EngineClient,
    EngineCrashed,
    EngineKilled,
    SlotUnavailable,
    next_message_id,
)
from worker.runtime.protocol import Call, Reply, SlotSpec
from worker.runtime.supervisor import (
    LANE_GROUPS,
    STATE_BROKEN,
    STATE_READY,
    STATE_RESTARTING,
    STATE_STOPPED,
    STATE_UNLOADED,
    EnginePlan,
    Supervisor,
    plan_engines,
)


def spec(name: str, replicas: int = 1) -> SlotSpec:
    return SlotSpec(name, "worker.models.x:Y", name, "rev", "cuda", 4, 30, {"replicas": replicas})


def test_plan_engines_slot_mode_one_process_per_replica() -> None:
    specs = {name: spec(name) for name in ("muq", "sep", "cpu-tools")}
    plans = plan_engines("slot", specs, {"muq": 1, "sep": 2, "cpu-tools": 1})
    assert [(plan.name, plan.slot_names) for plan in plans] == [
        ("muq", ("muq",)),
        ("sep#0", ("sep",)),
        ("sep#1", ("sep",)),
        ("cpu-tools", ("cpu-tools",)),
    ]


def test_plan_engines_lane_mode_groups_lane_models() -> None:
    names = ("muq", "mulan", "text", "sep", "asr", "align", "mms", "cpu-tools", "train-collab")
    specs = {name: spec(name) for name in names}
    replicas = {"sep": 2, "asr": 2, "align": 2, "mms": 1}
    plans = plan_engines("lane", specs, replicas, LANE_GROUPS)
    by_name = {plan.name: plan.slot_names for plan in plans}
    assert by_name == {
        "audio": ("muq", "mulan", "text"),
        "sync#0": ("sep", "asr", "align", "mms"),
        "sync#1": ("sep", "asr", "align"),
        "cpu-tools": ("cpu-tools",),
        "train-collab": ("train-collab",),
    }


def test_plan_engines_rejects_unknown_mode() -> None:
    with pytest.raises(ValueError, match="single"):
        plan_engines("single", {}, {})


async def call_once(
    supervisor: Supervisor,
    slot: str,
    method: str = "echo",
    args: dict[str, object] | None = None,
    deadline_s: float = 10.0,
) -> Reply:
    client = await supervisor.acquire(slot, time.monotonic() + deadline_s)
    call_id = next_message_id()
    blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
    try:
        call = Call(
            call_id,
            slot,
            method,
            time.monotonic() + deadline_s,
            blocks.share({"x": np.ones((1, 2), np.float32)}),
            args or {},
        )
        reply = await client.call(call)
        shm.take_all(reply.arrays)
        return reply
    finally:
        blocks.release()
        supervisor.release(client)


async def client_of(supervisor: Supervisor, slot: str) -> EngineClient:
    client = await supervisor.acquire(slot, time.monotonic() + 5.0)
    supervisor.release(client)
    return client


async def test_start_serves_slots_and_stop_reaps_every_process() -> None:
    plans = [EnginePlan("a", (fake_spec("a"),)), EnginePlan("b", (fake_spec("b"),))]
    supervisor = await started_supervisor(plans)
    pids = [pid for _, pid, _ in supervisor.engines()]
    assert supervisor.slot_state("a") == STATE_READY
    assert supervisor.slot_state("b") == STATE_READY
    assert all(process_alive(pid) for pid in pids)
    reply = await call_once(supervisor, "a", args={"tags": ["t"]})
    assert reply.ok and reply.result["tags"] == ["t!"]
    await supervisor.stop()
    for pid in pids:
        assert await process_gone(pid)
    assert supervisor.slot_state("a") == STATE_STOPPED
    assert not any(entry.name.startswith(f"wk-{os.getpid()}-") for entry in shm.SHM_DIR.iterdir())
    with pytest.raises(SlotUnavailable):
        await supervisor.acquire("a", time.monotonic() + 1)


async def test_crash_respawns_and_counts() -> None:
    counters = Counters()
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], counters=counters)
    try:
        [(_, first_pid, _)] = supervisor.engines()
        with pytest.raises(EngineCrashed):
            await call_once(supervisor, "a", "crash")
        assert await wait_until(lambda: supervisor.slot_state("a") == STATE_READY, 10.0)
        [(_, second_pid, _)] = supervisor.engines()
        assert second_pid != first_pid
        assert counters.value("slot_crashes_total", slot="a") == 1
        assert counters.value("slot_restarts_total", slot="a") == 1
        assert (await call_once(supervisor, "a")).ok
    finally:
        await supervisor.stop()


async def test_breaker_opens_after_repeated_sudden_deaths_then_recovers() -> None:
    counters = Counters()
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], counters=counters)
    try:
        for _ in range(TEST_POLICY.breaker_deaths):
            assert await wait_until(lambda: supervisor.slot_state("a") == STATE_READY, 10.0)
            with pytest.raises(EngineCrashed):
                await call_once(supervisor, "a", "crash")
        assert await wait_until(lambda: supervisor.slot_state("a") == STATE_BROKEN, 5.0)
        with pytest.raises(SlotUnavailable) as raised:
            await supervisor.acquire("a", time.monotonic() + 5)
        assert raised.value.state == STATE_BROKEN
        assert await wait_until(lambda: supervisor.slot_state("a") == STATE_READY, 10.0)
        assert counters.value("slot_crashes_total", slot="a") == TEST_POLICY.breaker_deaths
        assert (await call_once(supervisor, "a")).ok
    finally:
        await supervisor.stop()


async def test_deadline_kill_is_planned_and_respawns_immediately() -> None:
    counters = Counters()
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], counters=counters)
    try:
        started = time.monotonic()
        with pytest.raises(EngineKilled) as raised:
            await call_once(supervisor, "a", "hang", deadline_s=0.4)
        assert raised.value.cause == CAUSE_DEADLINE
        assert await wait_until(lambda: supervisor.slot_state("a") == STATE_READY, 10.0)
        assert time.monotonic() - started < 0.4 + 0.5 + 3.0
        assert counters.value("slot_kills_deadline_total", slot="a") == 1
        assert counters.value("slot_crashes_total", slot="a") == 0
        assert supervisor.snapshot()["a"]["kills_deadline"] == 1
    finally:
        await supervisor.stop()


async def test_ping_watchdog_replaces_unresponsive_engine() -> None:
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))])
    try:
        [(_, pid, _)] = supervisor.engines()
        os.kill(pid, signal.SIGSTOP)
        assert await process_gone(pid, TEST_POLICY.ping_interval_s + TEST_POLICY.ping_timeout_s + 5)
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, pid)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        assert (await call_once(supervisor, "a")).ok
    finally:
        await supervisor.stop()


@pytest.mark.parametrize("overlap", [True, False], ids=["overlap", "stop-then-spawn"])
async def test_recycle_after_calls(overlap: bool) -> None:
    counters = Counters()
    policy = replace(TEST_POLICY, recycle_after_calls=2, recycle_overlap=overlap)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a"),))], policy, counters=counters
    )
    try:
        [(_, first_pid, _)] = supervisor.engines()
        for _ in range(2):
            assert (await call_once(supervisor, "a")).ok
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, first_pid)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        assert await process_gone(first_pid)
        assert counters.value("slot_restarts_total", slot="a") == 1
        assert (await call_once(supervisor, "a")).ok
    finally:
        await supervisor.stop()


async def test_idle_unload_then_reload_on_demand() -> None:
    policy = replace(TEST_POLICY, idle_unload_s=0.3)
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], policy)
    try:
        assert await wait_until(lambda: supervisor.slot_state("a") == STATE_UNLOADED, 5.0)
        reply = await call_once(supervisor, "a", args={"tags": ["q"]})
        assert reply.ok and reply.result["tags"] == ["q!"]
        assert supervisor.slot_state("a") == STATE_READY
    finally:
        await supervisor.stop()


async def test_oom_report_unloads_idlest_other_slot_and_marks_recycle() -> None:
    policy = replace(TEST_POLICY, oom_recycle_count=2)
    plans = [EnginePlan("a", (fake_spec("a"),)), EnginePlan("b", (fake_spec("b"),))]
    supervisor = await started_supervisor(plans, policy)
    try:
        assert (await call_once(supervisor, "b")).ok
        assert (await call_once(supervisor, "a")).ok
        a = await client_of(supervisor, "a")
        supervisor.report_oom("a", a)
        assert await wait_until(lambda: supervisor.slot_state("b") == STATE_UNLOADED, 5.0)
        assert supervisor.slot_state("a") == STATE_READY
        [(_, a_pid, _), _] = supervisor.engines()
        supervisor.report_oom("a", a)
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, a_pid)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        assert supervisor.snapshot()["a"]["oom"] == 2
    finally:
        await supervisor.stop()


async def test_load_failure_ends_broken() -> None:
    policy = replace(TEST_POLICY, breaker_deaths=2)
    supervisor = Supervisor(
        [EnginePlan("a", (fake_spec("a", fail_load=True),))],
        policy,
        Counters(),
        launch=launch(),
        log=quiet_log(),
    )
    await supervisor.start()
    try:
        assert await settled(supervisor, ["a"]) == {"a": STATE_BROKEN}
    finally:
        await supervisor.stop()


async def test_acquire_waits_through_restart_and_honours_deadline() -> None:
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))])
    try:
        with pytest.raises(EngineCrashed):
            await call_once(supervisor, "a", "crash")
        assert supervisor.slot_state("a") in (STATE_RESTARTING, "loading")
        client: EngineClient = await supervisor.acquire("a", time.monotonic() + 10)
        supervisor.release(client)
        assert client.alive
    finally:
        await supervisor.stop()


async def test_acquire_is_not_stranded_by_a_ping_in_flight() -> None:
    policy = replace(TEST_POLICY, ping_interval_s=0.001)
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], policy)
    try:
        for _ in range(40):
            started = time.monotonic()
            client: EngineClient = await supervisor.acquire("a", started + 2)
            supervisor.release(client)
            assert time.monotonic() - started < 0.5
            await asyncio.sleep(0.002)
    finally:
        await supervisor.stop()


async def test_snapshot_lists_every_slot() -> None:
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))])
    try:
        assert (await call_once(supervisor, "a")).ok
        report = supervisor.snapshot()["a"]
        assert report["state"] == STATE_READY
        assert report["calls"] == 1
        assert report["restarts"] == 0
        assert set(report) == {
            "state",
            "restarts",
            "kills_deadline",
            "crashes",
            "oom",
            "reserved_gap_mib",
            "calls",
            "p50_ms",
            "p95_ms",
        }
    finally:
        await supervisor.stop()


async def test_idle_unload_timeout_kills_the_stuck_engine() -> None:
    policy = replace(TEST_POLICY, idle_unload_s=0.3, command_timeout_s=0.3)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", unload_delay_s=2.0),))], policy
    )
    try:
        [(_, pid, _)] = supervisor.engines()
        assert await process_gone(pid, 5.0)
        assert await wait_until(lambda: supervisor.engines()[0][1] not in (0, pid), 5.0)
    finally:
        await supervisor.stop()


async def test_crash_during_idle_unload_keeps_the_ping_watchdog() -> None:
    policy = replace(TEST_POLICY, idle_unload_s=1.0)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", unload_crash=True),))], policy
    )
    try:
        [(_, first, _)] = supervisor.engines()
        assert await process_gone(first, 5.0)
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, first)
                and supervisor.slot_state("a") == STATE_READY
            ),
            5.0,
        )
        [(_, second, _)] = supervisor.engines()
        os.kill(second, signal.SIGSTOP)
        watchdog_s = TEST_POLICY.ping_interval_s + TEST_POLICY.ping_timeout_s + 2.0
        assert await process_gone(second, watchdog_s)
    finally:
        await supervisor.stop()


async def test_oom_unload_timeout_kills_the_stuck_engine() -> None:
    policy = replace(TEST_POLICY, command_timeout_s=0.3, ping_timeout_s=5.0)
    plans = [
        EnginePlan("a", (fake_spec("a"),)),
        EnginePlan("b", (fake_spec("b", unload_delay_s=3.0),)),
    ]
    supervisor = await started_supervisor(plans, policy)
    try:
        assert (await call_once(supervisor, "b")).ok
        assert (await call_once(supervisor, "a")).ok
        [_, (_, b_pid, _)] = supervisor.engines()
        supervisor.report_oom("a", await client_of(supervisor, "a"))
        assert await process_gone(b_pid, 2.0)
    finally:
        await supervisor.stop()


async def unloaded_supervisor(load_delay_s: float) -> tuple[Supervisor, EngineClient]:
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", load_delay_s=load_delay_s),))]
    )
    client = await supervisor.acquire("a", time.monotonic() + 5)
    await client.unload("a", 5.0)
    supervisor.release(client)
    return supervisor, client


async def test_cancelled_load_on_demand_releases_the_engine() -> None:
    supervisor, client = await unloaded_supervisor(load_delay_s=1.0)
    try:
        waiting = asyncio.create_task(supervisor.acquire("a", time.monotonic() + 10))
        await asyncio.sleep(0.3)
        waiting.cancel()
        with pytest.raises(asyncio.CancelledError):
            await waiting
        again = await supervisor.acquire("a", time.monotonic() + 5)
        supervisor.release(again)
        assert again is client
    finally:
        await supervisor.stop()


async def test_acquire_honours_deadline_during_load_on_demand() -> None:
    supervisor, client = await unloaded_supervisor(load_delay_s=1.0)
    try:
        started = time.monotonic()
        with pytest.raises(DeadlineExceeded):
            await supervisor.acquire("a", started + 0.3)
        assert time.monotonic() - started < 0.8
        assert (await call_once(supervisor, "a")).ok
        assert supervisor.engines()[0][1] == client.pid
        with pytest.raises(DeadlineExceeded):
            await supervisor.acquire("a", time.monotonic() - 1)
        assert client.alive
    finally:
        await supervisor.stop()


async def test_recycle_does_not_orphan_an_engine_respawned_meanwhile() -> None:
    before = set(engine_children())
    policy = replace(TEST_POLICY, recycle_after_calls=1)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", load_delay_s=1.5),))], policy
    )
    try:
        assert (await call_once(supervisor, "a")).ok
        assert await wait_until(lambda: len(set(engine_children()) - before) == 2, 5.0)
        with pytest.raises(EngineKilled):
            await call_once(supervisor, "a", "hang", deadline_s=0.2)
        assert await wait_until(
            lambda: (
                len(set(engine_children()) - before) == 1
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
    finally:
        await supervisor.stop()
    assert await wait_until(lambda: not set(engine_children()) - before, 5.0)


async def test_recycle_lets_the_running_call_finish() -> None:
    before = set(engine_children())
    policy = replace(TEST_POLICY, recycle_after_calls=1)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", load_delay_s=1.0),))], policy
    )
    try:
        assert (await call_once(supervisor, "a")).ok
        assert await wait_until(lambda: len(set(engine_children()) - before) == 2, 5.0)
        reply = await call_once(supervisor, "a", args={"seconds": 4.0})
        assert reply.ok
    finally:
        await supervisor.stop()
    assert await wait_until(lambda: not set(engine_children()) - before, 5.0)


async def test_failed_oom_recycle_falls_back_to_sequential_restart(tmp_path: Path) -> None:
    marker = tmp_path / "fail-once"
    policy = replace(TEST_POLICY, oom_recycle_count=1, oom_unload=False)
    supervisor = await started_supervisor(
        [EnginePlan("a", (fake_spec("a", fail_load_once=str(marker)),))], policy
    )
    try:
        [(_, first, _)] = supervisor.engines()
        marker.touch()
        supervisor.report_oom("a", await client_of(supervisor, "a"))
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, first)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        assert not marker.exists()
    finally:
        await supervisor.stop()


async def test_recycle_does_not_block_the_ping_watchdog_of_other_engines() -> None:
    before = set(engine_children())
    policy = replace(TEST_POLICY, recycle_after_calls=1)
    plans = [
        EnginePlan("a", (fake_spec("a", load_delay_s=4.0),)),
        EnginePlan("b", (fake_spec("b"),)),
    ]
    supervisor = await started_supervisor(plans, policy)
    try:
        assert (await call_once(supervisor, "a")).ok
        assert await wait_until(lambda: len(set(engine_children()) - before) == 3, 5.0)
        [_, (_, b_pid, _)] = supervisor.engines()
        os.kill(b_pid, signal.SIGSTOP)
        watchdog_s = TEST_POLICY.ping_interval_s + TEST_POLICY.ping_timeout_s + 1.0
        assert await process_gone(b_pid, watchdog_s)
    finally:
        await supervisor.stop()


async def test_engine_that_survives_its_kill_is_replaced(monkeypatch: pytest.MonkeyPatch) -> None:
    counters = Counters()
    policy = replace(TEST_POLICY, kill_join_s=0.3)
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], policy, counters)
    [(_, stuck, _)] = supervisor.engines()
    real_killpg = os.killpg

    def killpg_sparing_the_stuck(pgid: int, sig: int) -> None:
        if pgid != stuck:
            real_killpg(pgid, sig)

    monkeypatch.setattr(os, "killpg", killpg_sparing_the_stuck)
    try:
        with pytest.raises(EngineKilled):
            await asyncio.wait_for(call_once(supervisor, "a", "hang", deadline_s=0.3), 3.0)
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, stuck)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        assert process_alive(stuck)
        assert counters.value("engine_kill_join_timeout_total", engine="a") == 1
        assert (await call_once(supervisor, "a")).ok
    finally:
        monkeypatch.undo()
        os.killpg(stuck, signal.SIGKILL)
        await supervisor.stop()
    assert await process_gone(stuck)


async def test_gpu_oom_unloads_a_gpu_slot_and_spares_the_idler_cpu_slot() -> None:
    plans = [
        EnginePlan("gpu", (fake_spec("gpu", device="cuda"),)),
        EnginePlan("gpu-idle", (fake_spec("gpu-idle", device="cuda"),)),
        EnginePlan("cpu-idle", (fake_spec("cpu-idle", device="cpu"),)),
    ]
    supervisor = Supervisor(
        plans,
        replace(TEST_POLICY, ping_interval_s=30.0),
        Counters(),
        launch=launch(CUDA_VISIBLE_DEVICES=""),
        log=quiet_log(),
    )
    await supervisor.start()
    try:
        await settled(supervisor, supervisor.slots)
        for slot in ("cpu-idle", "gpu-idle", "gpu"):
            assert (await call_once(supervisor, slot)).ok
        supervisor.report_oom("gpu", await client_of(supervisor, "gpu"))
        assert await wait_until(lambda: supervisor.slot_state("gpu-idle") == STATE_UNLOADED, 5.0)
        assert supervisor.slot_state("cpu-idle") == STATE_READY
        assert supervisor.slot_state("gpu") == STATE_READY
    finally:
        await supervisor.stop()


async def test_repeated_oom_recycles_only_the_replica_that_ran_out() -> None:
    policy = replace(TEST_POLICY, oom_recycle_count=2, oom_unload=False)
    plans = [EnginePlan("a#0", (fake_spec("a"),)), EnginePlan("a#1", (fake_spec("a"),))]
    supervisor = await started_supervisor(plans, policy)
    try:
        pids = [pid for _, pid, _ in supervisor.engines()]
        victim = await client_of(supervisor, "a")
        assert victim.pid in pids
        ran_out = pids.index(victim.pid)
        spared = 1 - ran_out
        supervisor.report_oom("a", victim)
        supervisor.report_oom("a", victim)
        assert await wait_until(
            lambda: (
                supervisor.engines()[ran_out][1] not in (0, pids[ran_out])
                and supervisor.engines()[ran_out][2] == STATE_READY
            ),
            10.0,
        )
        assert supervisor.engines()[spared][1] == pids[spared]
        await asyncio.sleep(policy.ping_interval_s * 3)
        assert supervisor.engines()[spared][1] == pids[spared]
    finally:
        await supervisor.stop()


async def test_oom_recycle_wish_dies_with_the_engine_that_made_it() -> None:
    policy = replace(TEST_POLICY, oom_recycle_count=1, oom_unload=False)
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], policy)
    try:
        [(_, first, _)] = supervisor.engines()
        client = await client_of(supervisor, "a")
        hanging = asyncio.create_task(call_once(supervisor, "a", "hang", deadline_s=0.6))
        assert await wait_until(lambda: client.busy, 2.0)
        supervisor.report_oom("a", client)
        with pytest.raises(EngineKilled):
            await hanging
        assert await wait_until(
            lambda: (
                supervisor.engines()[0][1] not in (0, first)
                and supervisor.slot_state("a") == STATE_READY
            ),
            10.0,
        )
        [(_, respawned, _)] = supervisor.engines()
        await asyncio.sleep(policy.ping_interval_s * 4)
        assert supervisor.engines()[0][1] == respawned
    finally:
        await supervisor.stop()
