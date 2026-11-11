from __future__ import annotations

import asyncio
import os
import time
from dataclasses import replace

import numpy as np

from tests.runtime.support import TEST_POLICY, fake_spec, started_supervisor
from worker.domain.deadline import Deadline
from worker.engines import EngineSlots
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.batcher import Batcher
from worker.runtime.engine_client import EngineClient, next_message_id
from worker.runtime.protocol import Call, Reply
from worker.runtime.supervisor import (
    BULK_NICE,
    CPU_LANE_GROUPS,
    EnginePlan,
    Supervisor,
    budget_plans,
    cpu_budget,
    cpus_within_pid_limit,
    fair_share,
    plan_engines,
    reserved_threads,
)

SHARED = replace(TEST_POLICY, threads=0, cpu_budget=6, share_window_s=0.0)
PROMPT_S = 1.0


def plans_of(*plans: EnginePlan) -> list[EnginePlan]:
    return list(budget_plans(plans, 8))


def test_engines_take_at_most_half_of_the_container_pid_limit() -> None:
    assert cpus_within_pid_limit(64, 13, 2560) == 33
    assert cpus_within_pid_limit(8, 13, 2560) == 8
    assert cpus_within_pid_limit(64, 13, None) == 64
    assert cpus_within_pid_limit(64, 200, 512) == 1


def test_the_budget_leaves_a_core_for_the_node_and_reserves_up_to_four_threads() -> None:
    assert [cpu_budget(cpus) for cpus in (1, 2, 4, 8, 32)] == [1, 2, 3, 7, 31]
    assert [reserved_threads(cpus) for cpus in (1, 2, 4, 8, 32)] == [1, 1, 2, 4, 4]
    assert [fair_share(7, active) for active in (0, 1, 2, 3, 8)] == [7, 7, 4, 2, 1]
    assert [fair_share(3, active) for active in (1, 2, 3)] == [3, 2, 1]


def test_bulk_engines_share_the_budget_and_inline_tools_take_one_thread() -> None:
    plans = budget_plans(
        [
            EnginePlan("audio", (fake_spec("muq"), fake_spec("mulan"))),
            EnginePlan("sep", (fake_spec("sep"),)),
            EnginePlan("train-taste", (fake_spec("train-taste"),)),
            EnginePlan("cpu-tools", (fake_spec("vad"), fake_spec("lid"), fake_spec("fingerprint"))),
            EnginePlan("encode", (fake_spec("text"), fake_spec("mulan")), reserved=True),
        ],
        8,
    )
    assert {plan.name: (plan.threads, plan.nice) for plan in plans} == {
        "audio": (0, BULK_NICE),
        "sep": (0, BULK_NICE),
        "train-taste": (0, BULK_NICE),
        "cpu-tools": (1, 0),
        "encode": (4, 0),
    }


def test_cpu_lane_groups_run_separation_and_text_in_their_own_processes() -> None:
    names = ("muq", "mulan", "text", "sep", "asr", "align", "mms")
    plans = plan_engines("lane", {name: fake_spec(name) for name in names}, {}, CPU_LANE_GROUPS)
    assert {plan.name: plan.slot_names for plan in plans} == {
        "audio": ("muq", "mulan"),
        "sync": ("asr", "align", "mms"),
        "text": ("text",),
        "sep": ("sep",),
    }


async def test_serial_loads_bring_engines_up_one_at_a_time() -> None:
    plans = [EnginePlan(name, (fake_spec(name, load_delay_s=0.6),)) for name in ("a", "b")]
    started = time.monotonic()
    parallel = await started_supervisor(plans, SHARED)
    parallel_s = time.monotonic() - started
    await parallel.stop()
    started = time.monotonic()
    serial = await started_supervisor(plans, replace(SHARED, serial_loads=True))
    serial_s = time.monotonic() - started
    await serial.stop()
    assert serial_s >= 1.2
    assert serial_s - parallel_s >= 0.4


async def test_busy_engines_split_the_budget_and_an_idle_one_takes_it_all() -> None:
    supervisor = await started_supervisor(
        plans_of(EnginePlan("a", (fake_spec("a"),)), EnginePlan("b", (fake_spec("b"),))), SHARED
    )
    try:
        first = await supervisor.acquire("a", time.monotonic() + 5)
        assert first.call_threads == 6
        second = await supervisor.acquire("b", time.monotonic() + 5)
        assert second.call_threads == 3
        supervisor.release(first)
        supervisor.release(second)
        alone = await supervisor.acquire("b", time.monotonic() + 5)
        assert alone.call_threads == 6
        supervisor.release(alone)
    finally:
        await supervisor.stop()


async def test_an_engine_that_just_had_work_keeps_its_share_until_the_window_passes() -> None:
    policy = replace(SHARED, share_window_s=0.5)
    supervisor = await started_supervisor(
        plans_of(EnginePlan("a", (fake_spec("a"),)), EnginePlan("b", (fake_spec("b"),))), policy
    )
    try:
        supervisor.release(await supervisor.acquire("a", time.monotonic() + 5))
        separation = await supervisor.acquire("b", time.monotonic() + 5)
        assert separation.call_threads == 3
        supervisor.release(separation)
        await asyncio.sleep(0.6)
        alone = await supervisor.acquire("b", time.monotonic() + 5)
        assert alone.call_threads == 6
        supervisor.release(alone)
    finally:
        await supervisor.stop()


async def test_encode_queries_keep_their_threads_and_lyrics_take_a_bulk_share() -> None:
    supervisor = await started_supervisor(
        plans_of(
            EnginePlan("audio", (fake_spec("a"),)),
            EnginePlan("encode", (fake_spec("text"),), reserved=True),
        ),
        SHARED,
    )
    try:
        query = await supervisor.acquire("text", time.monotonic() + 5, reserved=True)
        audio = await supervisor.acquire("a", time.monotonic() + 5)
        assert (query.call_threads, audio.call_threads) == (4, 6)
        supervisor.release(query)
        supervisor.release(audio)
        query = await supervisor.acquire("text", time.monotonic() + 5, priority=True)
        assert query.call_threads == 4
        supervisor.release(query)
        lyric = await supervisor.acquire("text", time.monotonic() + 5)
        assert lyric.call_threads == 6
        audio = await supervisor.acquire("a", time.monotonic() + 5)
        assert audio.call_threads == 3
        supervisor.release(lyric)
        supervisor.release(audio)
    finally:
        await supervisor.stop()


async def test_a_fixed_thread_count_wins_over_the_shared_budget() -> None:
    policy = replace(SHARED, threads=5)
    supervisor = await started_supervisor(
        plans_of(
            EnginePlan("a", (fake_spec("a"),)),
            EnginePlan("encode", (fake_spec("e"),), reserved=True),
        ),
        policy,
    )
    try:
        bulk = await supervisor.acquire("a", time.monotonic() + 5)
        reserved = await supervisor.acquire("e", time.monotonic() + 5, reserved=True)
        assert (bulk.call_threads, reserved.call_threads) == (5, 4)
        supervisor.release(bulk)
        supervisor.release(reserved)
    finally:
        await supervisor.stop()


async def test_call_threads_and_niceness_reach_the_engine_process() -> None:
    plan = plans_of(EnginePlan("a", (fake_spec("a", device="cpu"),)))
    supervisor = await started_supervisor(plan, SHARED)
    try:
        reply = await call(supervisor, "a", "threads")
        assert reply.ok, reply.error
        assert reply.result["threads"] == 6
        assert reply.result["nice"] >= BULK_NICE
    finally:
        await supervisor.stop()


async def test_an_engine_without_torch_still_learns_the_threads_of_each_call() -> None:
    supervisor = await started_supervisor(
        plans_of(
            EnginePlan("a", (fake_spec("a"),)),
            EnginePlan("llm-local", (fake_spec("llm-local"),), reserved=True),
        ),
        SHARED,
    )
    try:
        bulk = await call(supervisor, "a", "granted_threads")
        other = await supervisor.acquire("llm-local", time.monotonic() + 5)
        shared = await call(supervisor, "a", "granted_threads")
        supervisor.release(other)
        urgent = await call(supervisor, "llm-local", "granted_threads", priority=True)
        assert (bulk.result["threads"], shared.result["threads"]) == (6, 3)
        assert urgent.result["threads"] == reserved_threads(8) == 4
    finally:
        await supervisor.stop()


def test_a_plan_with_its_own_threads_and_nice_keeps_them_within_the_budget() -> None:
    plans = budget_plans(
        [
            EnginePlan("llm-local", (fake_spec("llm-local"),), reserved=True, threads=2, nice=5),
            EnginePlan("wide", (fake_spec("x"),), reserved=True, threads=64),
        ],
        8,
    )
    assert [(plan.threads, plan.nice) for plan in plans] == [(2, 5), (7, 0)]


async def test_a_running_local_model_call_takes_its_threads_out_of_the_bulk_share() -> None:
    model = EnginePlan(
        "llm-local", (fake_spec("llm-local"),), reserved=True, threads=2, shares_budget=True
    )
    supervisor = await started_supervisor(
        plans_of(EnginePlan("a", (fake_spec("a"),)), model), SHARED
    )
    try:
        before = await call(supervisor, "a", "granted_threads")
        leased = await supervisor.acquire("llm-local", time.monotonic() + 5, priority=True)
        during = await call(supervisor, "a", "granted_threads")
        supervisor.release(leased)
        after = await call(supervisor, "a", "granted_threads")
        assert leased.call_threads == 2
        assert [reply.result["threads"] for reply in (before, during, after)] == [6, 4, 6]
    finally:
        await supervisor.stop()


async def test_an_engine_runs_with_the_nice_and_env_of_its_plan() -> None:
    model = EnginePlan(
        "llm-local",
        (fake_spec("llm-local"),),
        reserved=True,
        threads=2,
        nice=5,
        env={"OMP_WAIT_POLICY": "PASSIVE"},
    )
    supervisor = await started_supervisor(
        plans_of(model, EnginePlan("a", (fake_spec("a"),))), SHARED
    )
    try:
        own = await call(supervisor, "llm-local", "environment", priority=True)
        bulk = await call(supervisor, "a", "environment")
        assert own.result["wait_policy"] == "PASSIVE"
        assert 5 <= int(str(own.result["nice"])) < BULK_NICE
        assert bulk.result["wait_policy"] == ""
    finally:
        await supervisor.stop()


async def test_encode_is_served_by_the_reserved_engine_while_the_bulk_one_is_busy() -> None:
    supervisor = await started_supervisor(
        plans_of(
            EnginePlan("audio", (fake_spec("text"),)),
            EnginePlan("encode", (fake_spec("text"),), reserved=True),
        ),
        SHARED,
    )
    counters = Counters()
    bulk = Batcher("text", 8, 0, supervisor.pool(reserved=False), counters)
    priority = Batcher("text", 8, 0, supervisor.pool(reserved=True), counters)
    slots = EngineSlots(supervisor, {"text": bulk}, {"text": 8}, lambda: None, {"text": priority})
    rows = {"x": np.ones((1, 2), np.float32)}
    try:
        busy = asyncio.create_task(
            slots.batched("text", "echo", rows, {"seconds": 3}, Deadline.after(30))
        )
        await asyncio.sleep(0.3)
        started = time.monotonic()
        _, result = await slots.batched("text", "echo", rows, {}, Deadline.after(30), priority=True)
        assert time.monotonic() - started < PROMPT_S
        _, busy_result = await busy
        assert result["pid"] != busy_result["pid"]
        pids = {name: pid for name, pid, _ in supervisor.engines()}
        assert (result["pid"], busy_result["pid"]) == (pids["encode"], pids["audio"])
    finally:
        await priority.close()
        await bulk.close()
        await supervisor.stop()


async def call(supervisor: Supervisor, slot: str, method: str, *, priority: bool = False) -> Reply:
    client: EngineClient = await supervisor.acquire(slot, time.monotonic() + 30, priority=priority)
    call_id = next_message_id()
    blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
    try:
        reply = await client.call(
            Call(
                call_id,
                slot,
                method,
                time.monotonic() + 30,
                blocks.share({"x": np.ones((1, 2), np.float32)}),
            )
        )
        shm.take_all(reply.arrays)
        return reply
    finally:
        blocks.release()
        supervisor.release(client)
