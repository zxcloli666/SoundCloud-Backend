from __future__ import annotations

import asyncio
import time

import numpy as np

from tests.runtime.support import fake_spec, quiet_log, started_supervisor
from worker.observability.counters import Counters
from worker.runtime.batcher import Batcher
from worker.runtime.supervisor import EnginePlan

ROW = {"x": np.ones((1, 2), np.float32)}
LYRIC = {"layers": 100}
QUERY_BUDGET_S = 1.0


def backlog(batcher: Batcher, size: int) -> list[asyncio.Task[object]]:
    deadline_at = time.monotonic() + 120.0
    return [
        asyncio.create_task(batcher.submit("layers", ROW, LYRIC, deadline_at)) for _ in range(size)
    ]


async def drain(tasks: list[asyncio.Task[object]]) -> None:
    for task in tasks:
        task.cancel()
    await asyncio.gather(*tasks, return_exceptions=True)


async def test_a_mulan_query_preempts_lyrics_on_the_shared_encode_engine() -> None:
    counters = Counters()
    supervisor = await started_supervisor(
        [EnginePlan("encode", (fake_spec("text"), fake_spec("mulan")), reserved=True)],
        counters=counters,
    )
    lyrics = Batcher("text", 1, 0, supervisor.pool(reserved=False), counters, log=quiet_log())
    queries = Batcher("mulan", 8, 0, supervisor.pool(reserved=True), counters, log=quiet_log())
    pending = backlog(lyrics, 12)
    try:
        latencies = []
        for _ in range(3):
            await asyncio.sleep(0.5)
            started = time.monotonic()
            await queries.submit("echo", ROW, {}, time.monotonic() + 10.0, priority=True)
            latencies.append(time.monotonic() - started)
        assert max(latencies) < QUERY_BUDGET_S
        assert counters.value("batch_preempted_total", slot="text") >= 3
    finally:
        await drain(pending)
        await lyrics.close()
        await queries.close()
        await supervisor.stop()


async def test_every_query_preempts_the_bulk_batch_running_at_that_moment() -> None:
    counters = Counters()
    supervisor = await started_supervisor(
        [EnginePlan("encode", (fake_spec("text"),), reserved=True)], counters=counters
    )
    lyrics = Batcher("text", 1, 0, supervisor.pool(reserved=False), counters, log=quiet_log())
    pending = backlog(lyrics, 4)
    try:
        latencies = []
        for _ in range(3):
            await asyncio.sleep(0.5)
            started = time.monotonic()
            await lyrics.submit("echo", ROW, {}, time.monotonic() + 10.0, priority=True)
            latencies.append(time.monotonic() - started)
        assert max(latencies) < QUERY_BUDGET_S
        assert counters.value("batch_preempted_total", slot="text") >= 3
        await asyncio.wait_for(asyncio.gather(*pending), 60.0)
    finally:
        await drain(pending)
        await lyrics.close()
        await supervisor.stop()
