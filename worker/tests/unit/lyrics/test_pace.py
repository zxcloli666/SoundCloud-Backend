from __future__ import annotations

from worker.domain.lyrics.pace import MISS_MARGIN, SKIP_DECAY, SMOOTHING, Pace


def test_an_unmeasured_pace_always_fits() -> None:
    assert Pace().fits(600.0, 1.0)


def test_the_first_run_sets_the_pace_and_later_runs_smooth_it() -> None:
    pace = Pace()
    pace.observe(200.0, 100.0)
    assert pace.seconds_per_audio_s == 0.5
    pace.observe(200.0, 300.0)
    assert pace.seconds_per_audio_s == 0.5 + SMOOTHING * (1.5 - 0.5)


def test_a_missed_budget_raises_the_pace_above_what_the_budget_allowed() -> None:
    pace = Pace(seconds_per_audio_s=0.5)
    pace.missed(200.0, 240.0, 0.0)
    assert pace.seconds_per_audio_s == MISS_MARGIN * 1.2
    assert not pace.fits(200.0, 240.0)
    assert pace.fits(100.0, 240.0)


def test_a_partial_run_teaches_the_pace_its_measured_rate() -> None:
    pace = Pace(seconds_per_audio_s=0.5)
    pace.missed(200.0, 240.0, 1000.0)
    assert pace.seconds_per_audio_s == 0.5 + SMOOTHING * (5.0 - 0.5)
    assert pace.seconds_per_audio_s > MISS_MARGIN * 1.2


def test_each_skip_lowers_the_pace_so_separation_is_tried_again() -> None:
    pace = Pace(seconds_per_audio_s=1.5)
    skips = 0
    while not pace.fits(200.0, 250.0):
        pace.skipped()
        skips += 1
    assert pace.seconds_per_audio_s <= 1.25
    assert skips == 4
    assert SKIP_DECAY < 1.0


def test_a_track_twice_over_budget_is_not_probed_again_for_many_skips() -> None:
    pace = Pace()
    pace.missed(210.0, 450.0, 900.0)
    skips = 0
    while not pace.fits(210.0, 450.0):
        pace.skipped()
        skips += 1
    assert skips >= 10
