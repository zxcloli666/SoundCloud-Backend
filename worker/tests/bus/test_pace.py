from __future__ import annotations

import pytest

from worker.bus.pace import HEADROOM, LanePace
from worker.contract import Contract


def test_a_fresh_lane_keeps_the_contract_deadline(contract: Contract) -> None:
    lane = contract.lane("audio")
    assert LanePace(lane).deadline_s() == lane.deadline_s


def test_slow_finished_tasks_stretch_the_deadline_to_their_pace(contract: Contract) -> None:
    lane = contract.lane("audio")
    pace = LanePace(lane)
    pace.finished(250.0)
    assert pace.deadline_s() == HEADROOM * 250.0


def test_fast_tasks_never_shrink_below_the_contract_deadline(contract: Contract) -> None:
    lane = contract.lane("audio")
    pace = LanePace(lane)
    pace.finished(5.0)
    assert pace.deadline_s() == lane.deadline_s


def test_expiries_push_the_deadline_up_but_never_past_half_the_attempt_window(
    contract: Contract,
) -> None:
    lane = contract.lane("audio")
    pace = LanePace(lane)
    for _ in range(40):
        pace.expired(pace.deadline_s())
    assert lane.attempt_window_s is not None
    assert pace.deadline_s() == pytest.approx(lane.attempt_window_s / 2)


def test_finishing_quickly_again_pulls_the_deadline_back(contract: Contract) -> None:
    lane = contract.lane("audio")
    pace = LanePace(lane)
    pace.finished(400.0)
    stretched = pace.deadline_s()
    for _ in range(20):
        pace.finished(30.0)
    assert pace.deadline_s() < stretched
