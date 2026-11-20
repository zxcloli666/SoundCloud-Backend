from __future__ import annotations

import numpy as np
import pytest

from eval import onsets, reference


def result(
    pairs: list[tuple[float, float, bool]], onset_s: list[float], fit: reference.Fit | None = None
) -> onsets.OnsetResult:
    return onsets.OnsetResult(tuple(pairs), tuple(onset_s), fit or reference.IDENTITY, {})


def test_nearest_distances_pick_the_closer_side() -> None:
    distances = onsets.nearest_distances_ms(
        np.array([0.9, 2.04, 5.0, 0.0]), np.array([3.0, 1.0, 2.0])
    )
    assert distances == pytest.approx([100.0, 40.0, 2000.0, 1000.0])


def test_no_onsets_gives_infinite_distance() -> None:
    distances = onsets.nearest_distances_ms(np.array([1.0]), np.zeros(0))
    assert np.isinf(distances[0])


def test_window_strength_takes_the_max_inside_the_window() -> None:
    envelope = np.zeros(100)
    envelope[50] = 0.8
    envelope[44] = 2.0
    strengths = onsets.window_strengths(np.array([0.48, 0.30, 0.55]), envelope)
    assert strengths == pytest.approx([0.8, 0.0, 0.0])


def test_window_strength_clips_at_the_edges() -> None:
    envelope = np.array([1.0, 0.0, 0.0, 0.5])
    strengths = onsets.window_strengths(np.array([-1.0, 5.0]), envelope)
    assert strengths == pytest.approx([1.0, 0.5])


def test_timeline_maps_reference_through_the_fit() -> None:
    fit = reference.Fit(1.0, 0.5, 1.0, True)
    pairs = [(10.0, fit.expected(9.5), False)]
    data = result(pairs, [10.0], fit)
    assert data.timeline("lrclib") == pytest.approx([10.0])
    assert data.timeline("ours", 100) == pytest.approx([10.1])
    assert data.timeline("lrclib", -100) == pytest.approx([9.9])


def test_fit_not_applied_keeps_raw_reference() -> None:
    fit = reference.Fit(1.0, 0.5, 0.2, False)
    assert fit.expected(9.5) == 9.5


def test_wins_count_ties_within_ten_ms() -> None:
    ours = np.array([10.0, 50.0, 30.0, 20.0])
    lrclib = np.array([100.0, 20.0, 35.0, 20.0])
    shares = onsets.wins(ours, lrclib)
    assert shares["ours"] == 0.25
    assert shares["lrclib"] == 0.25
    assert shares["tie"] == 0.5


def test_head_to_head_splits_large_disagreements() -> None:
    ours_ms = np.array([10.0, 10.0, 10.0])
    lrclib_ms = np.array([20.0, 130.0, 90.0])
    ours_s = np.array([1.0, 2.0, 3.0])
    lrclib_s = np.array([1.01, 2.2, 2.9])
    shares = onsets.head_to_head(ours_ms, lrclib_ms, ours_s, lrclib_s)
    disagree = shares["disagree_80ms"]
    assert disagree["n"] == 2
    assert disagree["ours"] == 1.0
    assert disagree["median_ours_gain_ms"] == 100.0
    assert shares["all"]["n"] == 3


def test_head_to_head_without_large_gaps() -> None:
    shares = onsets.head_to_head(np.array([5.0]), np.array([5.0]), np.array([1.0]), np.array([1.0]))
    assert shares["disagree_80ms"]["median_ours_gain_ms"] is None
    assert shares["disagree_80ms"]["ours"] is None


def clean_envelope() -> np.ndarray:
    envelope = np.zeros(1000)
    for frame in range(50, 1000, 100):
        envelope[frame] = 1.0
    return envelope


def judged(ours_shift_s: float, lrclib_shift_s: float | None = None) -> onsets.OnsetResult:
    envelope = clean_envelope()
    peaks = [frame * onsets.HOP_S for frame in range(50, 1000, 100)]
    other = ours_shift_s if lrclib_shift_s is None else lrclib_shift_s
    pairs = [(peak + ours_shift_s, peak + other, False) for peak in peaks]
    partial = result(pairs, peaks)
    sweep = {
        name: tuple(
            float(np.mean(onsets.window_strengths(partial.timeline(name, shift), envelope)))
            for shift in onsets.SHIFTS_MS
        )
        for name in onsets.TIMELINES
    }
    return onsets.OnsetResult(partial.pairs, partial.onsets_s, partial.fit, sweep)


def test_judge_discriminates_on_clean_peaks() -> None:
    summary = onsets.metrics([judged(0.0)])
    assert summary["discriminates"] == {"ours": True, "lrclib": True}
    assert summary["judge_valid"] is True
    assert summary["note"] is None
    assert summary["scores"]["ours"]["within_30ms"] == 1.0
    assert summary["scores"]["ours/plus"]["within_30ms"] == 0.0


def test_judge_flags_timelines_that_sit_on_the_controls() -> None:
    summary = onsets.metrics([judged(0.1)])
    assert summary["judge_valid"] is False
    assert summary["note"] == onsets.INVALID_NOTE


def test_sweep_finds_the_known_best_shift() -> None:
    summary = onsets.metrics([judged(0.06, 0.0)])
    ours = summary["shift_sweep"]["ours"]
    assert ours["by_median"]["shift_ms"] == -60
    assert ours["by_median"]["at_best"]["median_ms"] == 0.0
    assert ours["by_median"]["at_zero"]["median_ms"] == 60.0
    assert ours["by_strength"]["at_best"]["strength"] == 1.0
    assert ours["by_strength"]["at_zero"]["strength"] == 0.0
    assert summary["shift_sweep"]["lrclib"]["by_median"]["shift_ms"] == 0


def test_best_shift_distribution_over_tracks() -> None:
    summary = onsets.metrics([judged(0.06), judged(0.02), judged(0.04), judged(0.1)])
    spread = summary["ours_best_shift_ms"]["by_median"]
    assert spread["tracks"] == 4
    assert spread["median"] == -50.0
    assert spread["q25"] == -70.0
    assert spread["q75"] == -35.0


def test_ties_prefer_the_smaller_shift() -> None:
    flat = [0.5] * len(onsets.SHIFTS_MS)
    assert onsets.SHIFTS_MS[onsets.best_index(flat, maximize=True)] == 0


def test_json_round_trip() -> None:
    data = judged(0.0)
    again = onsets.from_json(onsets.to_json(data))
    assert again == data
    assert onsets.from_json(None) is None
