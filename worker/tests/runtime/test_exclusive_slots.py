from __future__ import annotations

import asyncio
import os
import time

import numpy as np
import pytest

from tests.runtime.support import fake_spec, started_supervisor, wait_until
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.engine_client import DeadlineExceeded, EngineClient, next_message_id
from worker.runtime.protocol import Call, Reply
from worker.runtime.supervisor import STATE_READY, STATE_UNLOADED, EnginePlan, Supervisor

PLANS = [
    EnginePlan("image-edit", (fake_spec("image-edit"),), exclusive=True),
    EnginePlan("image-to-video", (fake_spec("image-to-video"),), exclusive=True),
]


async def test_exclusive_engines_start_without_weights_and_load_on_the_first_task() -> None:
    supervisor = await started_supervisor([*PLANS, EnginePlan("clip", (fake_spec("clip"),))])
    try:
        assert supervisor.slot_state("image-edit") == STATE_UNLOADED
        assert supervisor.slot_state("image-to-video") == STATE_UNLOADED
        assert supervisor.slot_state("clip") == STATE_READY
        assert (await call_once(supervisor, "image-edit")).ok
        assert supervisor.slot_state("image-edit") == STATE_READY
        assert supervisor.slot_state("image-to-video") == STATE_UNLOADED
    finally:
        await supervisor.stop()


async def test_a_switch_unloads_the_other_engine_before_loading() -> None:
    counters = Counters()
    supervisor = await started_supervisor(PLANS, counters=counters)
    try:
        assert (await call_once(supervisor, "image-edit")).ok
        assert (await call_once(supervisor, "image-to-video")).ok
        assert supervisor.slot_state("image-edit") == STATE_UNLOADED
        assert supervisor.slot_state("image-to-video") == STATE_READY
        assert (await call_once(supervisor, "image-edit")).ok
        assert supervisor.slot_state("image-to-video") == STATE_UNLOADED
        assert counters.value("slot_switches_total", slot="image-edit") == 1
        assert counters.value("slot_switches_total", slot="image-to-video") == 1
    finally:
        await supervisor.stop()


async def test_the_other_lane_waits_for_the_running_task_and_finds_one_engine_loaded() -> None:
    supervisor = await started_supervisor(PLANS)
    try:
        edit = await supervisor.acquire("image-edit", time.monotonic() + 10)
        running = asyncio.create_task(call_on(supervisor, edit, "image-edit", {"seconds": 0.5}))
        waiting = asyncio.create_task(supervisor.acquire("image-to-video", time.monotonic() + 10))
        await asyncio.sleep(0.2)
        assert not waiting.done()
        assert (await running).ok
        video = await waiting
        try:
            assert not edit.loaded("image-edit")
            assert video.loaded("image-to-video")
        finally:
            supervisor.release(video)
    finally:
        await supervisor.stop()


async def test_a_wait_for_the_other_engine_still_honours_the_deadline() -> None:
    supervisor = await started_supervisor(PLANS)
    try:
        edit = await supervisor.acquire("image-edit", time.monotonic() + 10)
        try:
            with pytest.raises(DeadlineExceeded):
                await supervisor.acquire("image-to-video", time.monotonic() + 0.3)
            assert edit.loaded("image-edit")
        finally:
            supervisor.release(edit)
    finally:
        await supervisor.stop()


async def test_engines_outside_the_group_load_side_by_side() -> None:
    plans = [EnginePlan("a", (fake_spec("a"),)), EnginePlan("b", (fake_spec("b"),))]
    supervisor = await started_supervisor(plans)
    try:
        assert (await call_once(supervisor, "a")).ok
        assert (await call_once(supervisor, "b")).ok
        assert await wait_until(
            lambda: supervisor.slot_state("a") == supervisor.slot_state("b") == STATE_READY, 2.0
        )
    finally:
        await supervisor.stop()


async def call_once(supervisor: Supervisor, slot: str) -> Reply:
    client = await supervisor.acquire(slot, time.monotonic() + 10)
    return await call_on(supervisor, client, slot, {})


async def call_on(
    supervisor: Supervisor, client: EngineClient, slot: str, args: dict[str, object]
) -> Reply:
    call_id = next_message_id()
    blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
    try:
        refs = blocks.share({"x": np.ones((1, 2), np.float32)})
        reply = await client.call(Call(call_id, slot, "echo", time.monotonic() + 10, refs, args))
        shm.take_all(reply.arrays)
        return reply
    finally:
        blocks.release()
        supervisor.release(client)
