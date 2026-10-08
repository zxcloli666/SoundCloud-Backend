from __future__ import annotations

import asyncio
import os
import time

import numpy as np
import pytest

from tests.runtime.support import fake_spec, quiet_log, started_supervisor
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.batcher import STAGE_CALL, Batcher
from worker.runtime.engine_client import DeadlineExceeded, next_message_id
from worker.runtime.protocol import EXPIRY_MARGIN_S, Call, ErrorKind
from worker.runtime.supervisor import EnginePlan

ROW = {"x": np.ones((1, 2), np.float32)}
LONG_CALL = {"layers": 500}


async def test_a_preempted_bulk_lyric_expires_on_the_encode_engine_without_a_kill() -> None:
    counters = Counters()
    supervisor = await started_supervisor(
        [EnginePlan("encode", (fake_spec("text"),), reserved=True)], counters=counters
    )
    batcher = Batcher("text", 8, 0, supervisor.pool(reserved=False), counters, log=quiet_log())
    before = supervisor.engines()
    deadline_s = 3.0
    try:
        started = time.monotonic()
        lyric = asyncio.create_task(batcher.submit("layers", ROW, LONG_CALL, started + deadline_s))
        while not lyric.done():
            await asyncio.sleep(0.3)
            await batcher.submit("echo", ROW, {}, time.monotonic() + 5.0, priority=True)
        with pytest.raises(DeadlineExceeded) as raised:
            await lyric
        assert raised.value.stage == STAGE_CALL
        assert time.monotonic() - started < deadline_s
        assert counters.value("batch_preempted_total", slot="text") >= 2
        assert counters.value("batch_expired_total", slot="text") == 1
        assert counters.value("slot_kills_deadline_total", slot="text") == 0
        assert supervisor.engines() == before
    finally:
        await batcher.close()
        await supervisor.stop()


@pytest.mark.parametrize("slot", ["text", "muq", "mulan"])
async def test_a_late_call_returns_expired_and_keeps_the_engine(slot: str) -> None:
    counters = Counters()
    supervisor = await started_supervisor(
        [EnginePlan("engine", (fake_spec(slot),))], counters=counters
    )
    before = supervisor.engines()
    deadline_s = 2.0
    try:
        client = await supervisor.acquire(slot, time.monotonic() + 5.0)
        call_id = next_message_id()
        blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
        started = time.monotonic()
        try:
            reply = await client.call(
                Call(
                    call_id,
                    slot,
                    "layers",
                    started + deadline_s,
                    blocks.share(ROW),
                    LONG_CALL,
                )
            )
        finally:
            blocks.release()
            supervisor.release(client)
        assert reply.error_kind is ErrorKind.EXPIRED
        assert time.monotonic() - started < deadline_s - EXPIRY_MARGIN_S / 2
        assert counters.value("slot_kills_deadline_total", slot=slot) == 0
        assert supervisor.engines() == before
    finally:
        await supervisor.stop()
