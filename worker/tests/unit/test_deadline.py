from __future__ import annotations

import pytest

from tests.fakes.clock import FakeClock
from worker.domain.deadline import Deadline, MonotonicClock
from worker.domain.outcome import Reason, TransientFailure


def test_remaining_budget_and_expiry() -> None:
    clock = FakeClock(100.0)
    deadline = Deadline.after(30, clock.now)
    assert deadline.remaining() == 30
    assert deadline.budget(90) == 30
    assert deadline.budget(5) == 5
    assert not deadline.expired()
    clock.advance(29)
    deadline.check("download")
    clock.advance(1)
    assert deadline.expired() and deadline.remaining() == 0
    with pytest.raises(TransientFailure) as raised:
        deadline.check("separate")
    assert raised.value.reason is Reason.DEADLINE_EXCEEDED
    assert raised.value.detail == "stage=separate"


def test_minus_keeps_the_clock() -> None:
    clock = FakeClock(0.0)
    deadline = Deadline.after(10, clock.now).minus(1.5)
    assert deadline.remaining() == 8.5
    clock.advance(8.5)
    assert deadline.expired()


def test_from_epoch_ms_maps_wall_clock_to_monotonic() -> None:
    clock = FakeClock(500.0)
    deadline = Deadline.from_epoch_ms(
        1_700_000_020_000, wall=lambda: 1_700_000_000.0, now=clock.now
    )
    assert deadline.remaining() == pytest.approx(20.0)
    clock.advance(20)
    assert deadline.expired()


async def test_monotonic_clock_sleeps() -> None:
    clock = MonotonicClock()
    before = clock.now()
    await clock.sleep(0.01)
    assert clock.now() - before >= 0.009


async def test_fake_clock_wakes_sleepers_in_order() -> None:
    clock = FakeClock()
    order: list[str] = []

    async def sleeper(name: str, seconds: float) -> None:
        await clock.sleep(seconds)
        order.append(name)

    import asyncio

    tasks = [asyncio.create_task(sleeper("b", 2)), asyncio.create_task(sleeper("a", 1))]
    await asyncio.sleep(0)
    assert clock.sleeping == 2
    await clock.tick(1)
    assert order == ["a"]
    await clock.tick(1)
    assert order == ["a", "b"]
    await asyncio.gather(*tasks)
