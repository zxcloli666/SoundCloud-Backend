from __future__ import annotations

import numpy as np

from worker.domain.lyrics import regions
from worker.domain.ports import Span

RATE = regions.SAMPLE_RATE


def silence(seconds: float) -> np.ndarray:
    return np.zeros(int(seconds * RATE), dtype=np.float32)


def test_overlapping_and_touching_spans_are_merged() -> None:
    built = regions.build(
        [Span(1.0, 4.0), Span(3.5, 6.0), Span(6.0, 7.0), Span(9.0, 9.5)],
        silence(20),
        min_s=1.2,
        max_s=28.0,
    )
    assert [(r.start_s, r.end_s) for r in built] == [(1.0, 7.0)]


def test_spans_are_clamped_to_the_track_and_short_ones_dropped() -> None:
    built = regions.build([Span(-1.0, 2.0), Span(18.0, 30.0)], silence(20), min_s=1.2, max_s=28.0)
    assert [(r.start_s, r.end_s) for r in built] == [(0.0, 2.0), (18.0, 20.0)]


def test_long_region_is_cut_at_the_quietest_moment() -> None:
    signal = np.ones(int(60 * RATE), dtype=np.float32) * 0.5
    quiet = int(24.0 * RATE)
    signal[quiet : quiet + RATE // 2] = 0.0
    built = regions.build([Span(0.0, 60.0)], signal, min_s=1.2, max_s=28.0)
    assert len(built) == 3
    assert abs(built[0].end_s - 24.25) < 0.3
    assert all(region.duration_s <= 28.0 for region in built)
    assert built[-1].end_s == 60.0


def test_clip_returns_samples_and_offset() -> None:
    signal = np.arange(RATE * 10, dtype=np.float32)
    samples, offset = regions.clip(signal, 2.0, 3.0)
    assert offset == 2.0
    assert samples.shape[0] == RATE
    assert samples[0] == 2 * RATE
    samples, offset = regions.clip(signal, -1.0, 0.5)
    assert offset == 0.0 and samples.shape[0] == RATE // 2


def test_total_speech_sums_durations() -> None:
    built = regions.build([Span(0.0, 2.0), Span(5.0, 9.0)], silence(10), min_s=1.2, max_s=28.0)
    assert regions.total_speech_s(built) == 6.0
