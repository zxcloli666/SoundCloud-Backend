from __future__ import annotations

import time

import pytest

from worker.models.roformer import check_pace
from worker.runtime.protocol import CallExpired


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
