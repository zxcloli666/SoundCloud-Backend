from __future__ import annotations

from collections.abc import Mapping

from tests.bus.conftest import Harness, build_harness
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeNats
from worker.bus.lane_runner import IDLE_POLL_S, PULL_EXPIRES_S, LaneRunner
from worker.bus.turn import EngineTurn
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.settings import Settings

KEYFRAME = "video-keyframe"
SHOT = "video-shot"
HOLD_S = 900.0


def test_the_first_lane_takes_a_free_turn_and_the_other_waits() -> None:
    turn = EngineTurn(FakeClock(), HOLD_S)
    assert turn.claim(KEYFRAME)
    assert not turn.claim(SHOT)
    assert turn.owner == KEYFRAME
    assert turn.claim(KEYFRAME)


def test_a_release_hands_the_turn_to_the_waiting_lane() -> None:
    turn = EngineTurn(FakeClock(), HOLD_S)
    turn.claim(KEYFRAME)
    turn.claim(SHOT)
    turn.release(SHOT)
    assert turn.owner == KEYFRAME
    turn.release(KEYFRAME)
    assert turn.owner == SHOT
    assert not turn.claim(KEYFRAME)


def test_a_release_with_nobody_waiting_frees_the_turn() -> None:
    turn = EngineTurn(FakeClock(), HOLD_S)
    turn.claim(KEYFRAME)
    turn.release(KEYFRAME)
    assert turn.owner is None
    assert turn.claim(SHOT)


def test_the_owner_keeps_the_turn_past_the_hold_while_nobody_waits() -> None:
    clock = FakeClock()
    turn = EngineTurn(clock, HOLD_S)
    turn.claim(KEYFRAME)
    clock.advance(HOLD_S * 3)
    assert turn.claim(KEYFRAME)
    turn.claim(SHOT)
    assert not turn.claim(KEYFRAME)


async def test_queued_tasks_of_the_owner_lane_run_before_a_switch(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = EngineTurn(clock, HOLD_S)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    shot = await turned_harness(fake_nats, contract, settings, clock, SHOT, turn, order)
    try:
        for step in (1, 2, 3):
            keyframe.enqueue({**samples.REQUESTS[KEYFRAME], "step": step}, f"kf:{step}")
        shot.enqueue(samples.REQUESTS[SHOT], "shot:1")
        keyframe.processor.hold()
        await start(keyframe)
        await start(shot)
        assert turn.owner == KEYFRAME
        assert (len(keyframe.processor.calls), shot.processor.calls) == (1, [])
        keyframe.processor.resume()
        await idle_polls(clock)
        assert order == [KEYFRAME, KEYFRAME, KEYFRAME, SHOT]
    finally:
        await keyframe.stop()
        await shot.stop()


async def test_a_turn_held_past_the_hold_passes_to_the_waiting_lane(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = EngineTurn(clock, HOLD_S)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    shot = await turned_harness(fake_nats, contract, settings, clock, SHOT, turn, order)
    try:
        for step in (1, 2):
            keyframe.enqueue({**samples.REQUESTS[KEYFRAME], "step": step}, f"kf:{step}")
        shot.enqueue(samples.REQUESTS[SHOT], "shot:1")
        keyframe.processor.hold()
        await start(keyframe)
        await start(shot)
        for _ in range(int(HOLD_S // keyframe.lane.heartbeat_s) + 1):
            await clock.tick(keyframe.lane.heartbeat_s)
        assert shot.processor.calls == []
        keyframe.processor.resume()
        await idle_polls(clock)
        assert order == [KEYFRAME, SHOT, KEYFRAME]
    finally:
        await keyframe.stop()
        await shot.stop()


async def turned_harness(
    fake_nats: FakeNats,
    contract: Contract,
    settings: Settings,
    clock: FakeClock,
    lane: str,
    turn: EngineTurn,
    order: list[str],
) -> Harness:
    harness = await build_harness(fake_nats, contract, settings, clock, lane, capacity=1)

    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        order.append(lane)
        raise PermanentFailure(Reason.INVALID_REQUEST, "scripted")

    harness.processor.behaviour = behaviour
    harness.runner = LaneRunner(
        harness.lane,
        harness.watch,
        harness.handler,
        harness.connection.js,
        harness.connection,
        harness.outbox,
        1,
        harness.counters,
        clock,
        turn,
    )
    await harness.watch.check()
    return harness


async def start(harness: Harness) -> None:
    harness.start()
    await harness.settle()


async def idle_polls(clock: FakeClock) -> None:
    for _ in range(12):
        await clock.tick(PULL_EXPIRES_S + IDLE_POLL_S, turns=40)
