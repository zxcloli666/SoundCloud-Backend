from __future__ import annotations

from collections.abc import Mapping

from tests.bus.conftest import AUDIO_TASK, Harness, build_harness
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeConsumer, FakeNats
from worker.bus.lane_runner import IDLE_POLL_S, PULL_EXPIRES_S, LaneRunner
from worker.bus.turn import EngineTurn
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.settings import Settings

AUDIO = "audio"
LYRICS = "lyrics"
KEYFRAME = "video-keyframe"
SHOT = "video-shot"
HOLD_S = 900.0
PROBE_S = 10.0
VIDEO_PAIR = {KEYFRAME: "keyframe", SHOT: "shot"}
MIXED = {AUDIO: "regular", LYRICS: "regular", **VIDEO_PAIR}


def video_pair(clock: FakeClock) -> EngineTurn:
    return EngineTurn(clock, HOLD_S, PROBE_S, VIDEO_PAIR)


def mixed(clock: FakeClock) -> EngineTurn:
    return EngineTurn(clock, HOLD_S, PROBE_S, MIXED, "regular")


def test_a_lane_without_work_never_takes_or_queues_for_the_turn() -> None:
    turn = video_pair(FakeClock())
    assert turn.needs_proof(KEYFRAME) and turn.needs_proof(SHOT)
    assert not turn.claim(KEYFRAME, wanted=False)
    assert not turn.claim(SHOT, wanted=False)
    assert turn.owner is None


def test_the_first_lane_with_work_takes_a_free_turn_and_the_other_waits() -> None:
    turn = video_pair(FakeClock())
    assert turn.claim(KEYFRAME, wanted=True)
    assert not turn.claim(SHOT, wanted=True)
    assert turn.owner == "keyframe"
    assert turn.needs_proof(KEYFRAME)


def test_the_owner_keeps_the_turn_while_it_has_work_and_passes_it_when_done() -> None:
    turn = video_pair(FakeClock())
    turn.claim(KEYFRAME, wanted=True)
    turn.claim(SHOT, wanted=True)
    turn.release(KEYFRAME)
    assert turn.owner == "shot"
    assert not turn.claim(KEYFRAME, wanted=True)


def test_an_owner_with_nobody_waiting_keeps_the_turn_and_pulls_without_proof() -> None:
    clock = FakeClock()
    turn = video_pair(clock)
    turn.claim(KEYFRAME, wanted=True)
    turn.release(KEYFRAME)
    clock.advance(HOLD_S * 3)
    assert turn.owner == "keyframe"
    assert not turn.needs_proof(KEYFRAME)
    assert turn.claim(KEYFRAME, wanted=True)


def test_the_regular_lanes_hold_the_turn_by_default_and_idle_polls_never_move_it() -> None:
    clock = FakeClock()
    turn = mixed(clock)
    assert not turn.needs_proof(AUDIO)
    for _ in range(100):
        assert turn.claim(AUDIO, wanted=True)
        assert turn.claim(LYRICS, wanted=True)
        turn.release(AUDIO)
        turn.release(LYRICS)
        assert not turn.claim(KEYFRAME, wanted=False)
        clock.advance(PROBE_S)
    assert turn.owner == "regular"


def test_a_waiting_video_job_takes_the_turn_once_the_regular_lanes_have_no_work() -> None:
    turn = mixed(FakeClock())
    turn.claim(AUDIO, wanted=True)
    turn.claim(LYRICS, wanted=True)
    assert not turn.claim(KEYFRAME, wanted=True)
    assert turn.needs_proof(AUDIO)
    assert not turn.claim(AUDIO, wanted=False)
    turn.release(AUDIO)
    assert turn.owner == "regular"
    turn.release(LYRICS)
    assert turn.owner == "keyframe"


def test_busy_regular_lanes_keep_the_turn_until_the_hold_runs_out() -> None:
    clock = FakeClock()
    turn = mixed(clock)
    turn.claim(AUDIO, wanted=True)
    turn.claim(KEYFRAME, wanted=True)
    clock.advance(HOLD_S - 1)
    assert turn.claim(AUDIO, wanted=True)
    clock.advance(1)
    assert not turn.claim(AUDIO, wanted=True)
    turn.release(AUDIO)
    assert turn.owner == "keyframe"


def test_the_regular_lanes_get_the_turn_back_as_soon_as_the_video_jobs_end() -> None:
    turn = mixed(FakeClock())
    turn.claim(KEYFRAME, wanted=True)
    assert not turn.claim(AUDIO, wanted=True)
    assert turn.needs_proof(KEYFRAME)
    assert not turn.claim(KEYFRAME, wanted=False)
    turn.release(KEYFRAME)
    assert turn.owner == "regular"
    assert turn.claim(AUDIO, wanted=True)


async def test_queued_tasks_of_the_owner_lane_run_before_a_switch(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = video_pair(clock)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    shot = await turned_harness(fake_nats, contract, settings, clock, SHOT, turn, order)
    try:
        for step in (1, 2, 3):
            keyframe.enqueue({**samples.REQUESTS[KEYFRAME], "step": step}, f"kf:{step}")
        keyframe.processor.hold()
        await start(keyframe)
        await clock.tick(PROBE_S, turns=40)
        shot.enqueue(samples.REQUESTS[SHOT], "shot:1")
        await start(shot)
        await clock.tick(PROBE_S, turns=40)
        assert turn.owner == "keyframe"
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
    turn = video_pair(clock)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    shot = await turned_harness(fake_nats, contract, settings, clock, SHOT, turn, order)
    try:
        for step in (1, 2):
            keyframe.enqueue({**samples.REQUESTS[KEYFRAME], "step": step}, f"kf:{step}")
        keyframe.processor.hold()
        await start(keyframe)
        await clock.tick(PROBE_S, turns=40)
        shot.enqueue(samples.REQUESTS[SHOT], "shot:1")
        await start(shot)
        for _ in range(int(HOLD_S // PROBE_S) + 1):
            await clock.tick(PROBE_S)
        assert shot.processor.calls == []
        keyframe.processor.resume()
        await idle_polls(clock)
        assert order == [KEYFRAME, SHOT, KEYFRAME]
    finally:
        await keyframe.stop()
        await shot.stop()


async def test_idle_lanes_leave_the_turn_where_it_is(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = mixed(clock)
    lanes = [
        await turned_harness(fake_nats, contract, settings, clock, lane, turn, order)
        for lane in (AUDIO, KEYFRAME, SHOT)
    ]
    owners: set[str | None] = set()
    try:
        for harness in lanes:
            await start(harness)
        for _ in range(60):
            await clock.tick(PROBE_S / 2, turns=40)
            owners.add(turn.owner)
        assert owners == {"regular"}
        assert order == []
    finally:
        for harness in lanes:
            await harness.stop()


async def test_a_video_job_takes_the_idle_card_and_the_regular_lane_resumes_after_it(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = mixed(clock)
    audio = await turned_harness(fake_nats, contract, settings, clock, AUDIO, turn, order)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    try:
        await start(audio)
        await start(keyframe)
        await idle_polls(clock)
        keyframe.enqueue(samples.REQUESTS[KEYFRAME], "kf:1")
        await idle_polls(clock)
        assert order == [KEYFRAME]
        audio.enqueue(AUDIO_TASK, "audio:1")
        await idle_polls(clock)
        assert order == [KEYFRAME, AUDIO]
        assert turn.owner == "regular"
    finally:
        await audio.stop()
        await keyframe.stop()


async def test_regular_tasks_in_flight_finish_before_the_video_job_starts(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = mixed(clock)
    audio = await turned_harness(fake_nats, contract, settings, clock, AUDIO, turn, order)
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    try:
        audio.enqueue(AUDIO_TASK, "audio:1")
        audio.processor.hold()
        await start(audio)
        await clock.tick(PROBE_S, turns=40)
        keyframe.enqueue(samples.REQUESTS[KEYFRAME], "kf:1")
        await start(keyframe)
        for _ in range(10):
            await clock.tick(PROBE_S, turns=40)
        assert turn.owner == "regular"
        assert keyframe.processor.calls == []
        audio.processor.resume()
        await idle_polls(clock)
        assert order == [AUDIO, KEYFRAME]
        subjects = [subject for subject, _, _ in audio.done_published()]
        assert subjects == [audio.lane.done_subject, keyframe.lane.done_subject]
    finally:
        await audio.stop()
        await keyframe.stop()


async def test_a_video_job_gets_the_card_within_the_hold_while_the_regular_lanes_stay_busy(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = mixed(clock)
    busy = [
        await turned_harness(
            fake_nats, contract, settings, clock, lane, turn, order, capacity=4, task_s=PROBE_S * 3
        )
        for lane in (AUDIO, LYRICS)
    ]
    keyframe = await turned_harness(fake_nats, contract, settings, clock, KEYFRAME, turn, order)
    try:
        for harness in busy:
            for index in range(int(HOLD_S // PROBE_S) * 4):
                harness.enqueue(samples.REQUESTS[harness.lane.name], f"{harness.lane.name}:{index}")
            await start(harness)
        await clock.tick(PROBE_S, turns=40)
        keyframe.enqueue(samples.REQUESTS[KEYFRAME], "kf:1")
        await start(keyframe)
        asked = clock.now()
        while KEYFRAME not in order and clock.now() - asked < HOLD_S * 2:
            await clock.tick(PROBE_S / 2, turns=40)
        assert KEYFRAME in order
        assert clock.now() - asked <= HOLD_S + PROBE_S * 4
        assert {AUDIO, LYRICS} <= set(order[: order.index(KEYFRAME)])
        assert all(consumer(harness).num_pending(clock.now()) for harness in busy)
    finally:
        for harness in (*busy, keyframe):
            await harness.stop()


async def test_a_video_job_takes_the_turn_on_a_broker_that_only_shows_work_to_a_puller(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    order: list[str] = []
    turn = mixed(clock)
    audio = await turned_harness(
        fake_nats, contract, settings, clock, AUDIO, turn, order, capacity=4, task_s=PROBE_S * 3
    )
    shot = await turned_harness(fake_nats, contract, settings, clock, SHOT, turn, order)
    try:
        for index in range(int(HOLD_S // PROBE_S) * 4):
            audio.enqueue(samples.REQUESTS[AUDIO], f"audio:{index}")
        await start(audio)
        await clock.tick(PROBE_S, turns=40)
        shot.enqueue(samples.REQUESTS[SHOT], "shot:1")
        consumer(shot).num_pending = lambda now: 0
        await start(shot)
        asked = clock.now()
        while SHOT not in order and clock.now() - asked < HOLD_S * 2:
            await clock.tick(PROBE_S / 2, turns=40)
        assert SHOT in order
        assert clock.now() - asked <= HOLD_S + PROBE_S * 4
        await idle_polls(clock)
        assert order[-1] == AUDIO
    finally:
        await audio.stop()
        await shot.stop()


async def turned_harness(
    fake_nats: FakeNats,
    contract: Contract,
    settings: Settings,
    clock: FakeClock,
    lane: str,
    turn: EngineTurn,
    order: list[str],
    capacity: int = 1,
    task_s: float = 0.0,
) -> Harness:
    harness = await build_harness(fake_nats, contract, settings, clock, lane, capacity=capacity)

    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        order.append(lane)
        await clock.sleep(task_s)
        raise PermanentFailure(Reason.INVALID_REQUEST, "scripted")

    harness.processor.behaviour = behaviour
    harness.runner = LaneRunner(
        harness.lane,
        harness.watch,
        harness.handler,
        harness.connection.js,
        harness.connection,
        harness.outbox,
        capacity,
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


def consumer(harness: Harness) -> FakeConsumer:
    return harness.bus.consumers[(harness.lane.stream, harness.lane.durable)]
