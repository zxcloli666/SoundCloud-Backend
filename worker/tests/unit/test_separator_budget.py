from __future__ import annotations

import time

import pytest

from worker.domain.outcome import Reason, TransientFailure
from worker.engines import separation_expired
from worker.models.roformer import check_pace
from worker.runtime.engine_client import EngineError, EngineKilled
from worker.runtime.protocol import CallExpired, ErrorKind


def test_the_first_chunk_runs_while_the_budget_lasts() -> None:
    now = time.monotonic()
    check_pace(now, 0, 30, now + 1.0)
    with pytest.raises(CallExpired, match="after 0 of 30 chunks"):
        check_pace(now - 2.0, 0, 30, now - 1.0)


def test_a_projected_finish_past_the_budget_stops_after_the_first_chunks() -> None:
    now = time.monotonic()
    with pytest.raises(CallExpired, match="after 2 of 30 chunks"):
        check_pace(now - 20.0, 2, 30, now + 100.0)


def test_a_pace_that_fits_keeps_separating() -> None:
    now = time.monotonic()
    check_pace(now - 20.0, 10, 30, now + 100.0)


def test_an_expiry_reports_its_progress_and_the_engine_budget() -> None:
    now = time.monotonic()
    with pytest.raises(CallExpired) as raised:
        check_pace(now - 20.0, 2, 30, now + 100.0)
    assert raised.value.details["chunks"] == 2
    assert raised.value.details["total_chunks"] == 30
    assert raised.value.details["elapsed_s"] == pytest.approx(20.0, abs=1.0)
    assert raised.value.details["budget_s"] == pytest.approx(120.0, abs=1.0)


def test_an_expired_separation_call_keeps_its_progress_for_the_domain() -> None:
    cause = EngineError(ErrorKind.EXPIRED, "after 0 of 9 chunks", {"chunks": 0, "budget_s": 0.0})
    failure = TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=sep after 0 of 9 chunks")
    failure.__cause__ = cause
    expired = separation_expired(failure)
    assert expired is not None
    assert (expired.chunks, expired.budget_s, expired.reason) == (
        0,
        0.0,
        Reason.DEADLINE_EXCEEDED,
    )


def test_a_partial_separation_projects_the_full_run_from_its_chunks() -> None:
    details = {"chunks": 3, "total_chunks": 30, "elapsed_s": 45.0, "budget_s": 300.0}
    failure = TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=sep after 3 of 30 chunks")
    failure.__cause__ = EngineError(ErrorKind.EXPIRED, "after 3 of 30 chunks", details)
    expired = separation_expired(failure)
    assert expired is not None
    assert (expired.total_chunks, expired.elapsed_s) == (30, 45.0)
    assert expired.projected_s == 450.0


def test_a_killed_separation_call_is_not_an_expiry() -> None:
    failure = TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=sep killed=deadline")
    failure.__cause__ = EngineKilled("sep", "deadline")
    assert separation_expired(failure) is None
