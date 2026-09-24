from __future__ import annotations

import asyncio
import os
import time

import numpy as np
import pytest

from tests.runtime.support import fake_spec, process_gone, quiet_log, started_supervisor
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.batcher import Batcher
from worker.runtime.engine_client import DeadlineExceeded
from worker.runtime.supervisor import STATE_READY, EnginePlan


async def test_innocent_neighbour_survives_deadline_kill_in_the_same_delivery() -> None:
    counters = Counters()
    supervisor = await started_supervisor([EnginePlan("a", (fake_spec("a"),))], counters=counters)
    b = Batcher("a", 8, 30, supervisor, counters, log=quiet_log())
    try:
        [(_, first_pid, _)] = supervisor.engines()
        marker = shm.engine_prefix(os.getpid(), first_pid) + "stale"
        (shm.SHM_DIR / marker).write_bytes(b"\0" * 8)
        started = time.monotonic()
        deadline_at = started + 0.6
        poison = asyncio.create_task(
            b.submit(
                "echo",
                {"x": np.ones((1, 2), np.float32)},
                {"hang": [True], "tags": ["p"]},
                deadline_at,
            )
        )
        innocent = asyncio.create_task(
            b.submit(
                "echo",
                {"x": np.full((2, 2), 5, np.float32)},
                {"hang": [False, False], "tags": ["i1", "i2"]},
                started + 10,
            )
        )
        with pytest.raises(DeadlineExceeded):
            await poison
        killed_at = time.monotonic()
        assert deadline_at <= killed_at <= deadline_at + 0.5
        arrays, result = await innocent
        np.testing.assert_array_equal(arrays["x"], np.full((2, 2), 10, np.float32))
        assert result["tags"] == ["i1!", "i2!"]
        assert result["pid"] != first_pid
        assert not (shm.SHM_DIR / marker).exists()
        assert await process_gone(first_pid)
        assert counters.value("slot_kills_deadline_total", slot="a") == 1
        assert counters.value("slot_restarts_total", slot="a") == 1
        assert supervisor.slot_state("a") == STATE_READY
    finally:
        await b.close()
        await supervisor.stop()


async def test_collab_during_long_taste_both_finish() -> None:
    counters = Counters()
    plans = [
        EnginePlan("train-collab", (fake_spec("train-collab", max_batch=1),)),
        EnginePlan("train-taste", (fake_spec("train-taste", max_batch=1),)),
    ]
    supervisor = await started_supervisor(plans, counters=counters)
    collab = Batcher("train-collab", 1, 0, supervisor, counters, log=quiet_log())
    taste = Batcher("train-taste", 1, 0, supervisor, counters, log=quiet_log())
    try:
        taste_task = asyncio.create_task(
            taste.submit("echo", {}, {"seconds": 1.2, "tags": ["t"]}, time.monotonic() + 10)
        )
        await asyncio.sleep(0.2)
        collab_started = time.monotonic()
        _, collab_result = await collab.submit("echo", {}, {"tags": ["c"]}, time.monotonic() + 5)
        assert time.monotonic() - collab_started < 1.0
        _, taste_result = await taste_task
        assert collab_result["tags"] == ["c!"]
        assert taste_result["tags"] == ["t!"]
        assert counters.value("slot_restarts_total", slot="train-taste") == 0
        assert counters.value("slot_kills_deadline_total", slot="train-taste") == 0
    finally:
        await collab.close()
        await taste.close()
        await supervisor.stop()


async def test_two_replicas_serve_calls_in_parallel() -> None:
    counters = Counters()
    plans = [EnginePlan("a#0", (fake_spec("a"),)), EnginePlan("a#1", (fake_spec("a"),))]
    supervisor = await started_supervisor(plans, counters=counters)
    b = Batcher("a", 1, 0, supervisor, counters, log=quiet_log())
    try:
        started = time.monotonic()
        results = await asyncio.gather(
            *(
                b.submit("echo", {}, {"seconds": 0.4, "tags": [str(i)]}, time.monotonic() + 10)
                for i in range(2)
            )
        )
        assert time.monotonic() - started < 0.75
        assert {result["pid"] for _, result in results} == {
            pid for _, pid, _ in supervisor.engines()
        }
    finally:
        await b.close()
        await supervisor.stop()
