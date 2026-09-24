from __future__ import annotations

import asyncio

from tests.bus.conftest import AUDIO_TASK, Harness
from worker.bus.lease import Settled


async def two_leases(harness: Harness):
    harness.enqueue(AUDIO_TASK, None)
    harness.enqueue(AUDIO_TASK, None)
    first, second = await harness.fetch(batch=2)
    correlation = harness.lane.correlation_key(AUDIO_TASK)
    lease_a = harness.leases.attach(first, AUDIO_TASK, correlation).lease
    lease_b = harness.leases.attach(second, AUDIO_TASK, correlation).lease
    assert lease_a.stream_seq != lease_b.stream_seq
    return lease_a, lease_b


async def test_join_waits_for_the_owner_and_reports_published(harness: Harness) -> None:
    lease_a, lease_b = await two_leases(harness)
    assert await harness.inflight.join(lease_a) is None
    joined = asyncio.create_task(harness.inflight.join(lease_b))
    await harness.settle()
    assert not joined.done()
    assert len(harness.inflight) == 1
    lease_a.settle(Settled.PUBLISHED)
    assert await joined is Settled.PUBLISHED


async def test_join_takes_over_when_the_owner_nacks_or_drops(harness: Harness) -> None:
    for settled in (Settled.NACKED, Settled.DROPPED):
        lease_a, lease_b = await two_leases(harness)
        assert await harness.inflight.join(lease_a) is None
        joined = asyncio.create_task(harness.inflight.join(lease_b))
        await harness.settle()
        lease_a.settle(settled)
        assert await joined is None
        assert len(harness.inflight) == 1
        lease_b.settle(Settled.PUBLISHED)
        await harness.settle()
        assert len(harness.inflight) == 0


async def test_same_seq_joins_itself(harness: Harness) -> None:
    lease_a, _ = await two_leases(harness)
    assert await harness.inflight.join(lease_a) is None
    assert await harness.inflight.join(lease_a) is None
    assert len(harness.inflight) == 1
