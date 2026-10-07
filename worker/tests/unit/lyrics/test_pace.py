from __future__ import annotations

from worker.domain.lyrics.pace import SKIP_DECAY, SMOOTHING, Pace


def test_an_unmeasured_pace_always_fits() -> None:
    assert Pace().fits(600.0, 1.0)


def test_the_first_run_sets_the_pace_and_later_runs_smooth_it() -> None:
    pace = Pace()
    pace.observe(200.0, 100.0)
    assert pace.seconds_per_audio_s == 0.5
    pace.observe(200.0, 300.0)
    assert pace.seconds_per_audio_s == 0.5 + SMOOTHING * (1.5 - 0.5)


def test_a_missed_budget_raises_the_pace_to_at_least_what_was_spent() -> None:
    pace = Pace(seconds_per_audio_s=0.5)
    pace.missed(200.0, 300.0)
    assert pace.seconds_per_audio_s == 1.5
    assert not pace.fits(200.0, 250.0)
    assert pace.fits(100.0, 250.0)


def test_each_skip_lowers_the_pace_so_separation_is_tried_again() -> None:
    pace = Pace(seconds_per_audio_s=1.5)
    skips = 0
    while not pace.fits(200.0, 250.0):
        pace.skipped()
        skips += 1
    assert pace.seconds_per_audio_s <= 1.25
    assert skips == 2
    assert SKIP_DECAY < 1.0
