from __future__ import annotations

import numpy as np
import pytest

from worker.domain.audio import windows

RATE = 100


def track(seconds: int, loud: list[tuple[int, int]]) -> np.ndarray:
    signal = np.full(seconds * RATE, 0.001, dtype=np.float32)
    for start, end in loud:
        signal[start * RATE : end * RATE] = 0.5
    return signal


def starts(chosen: list[windows.Window]) -> list[float]:
    return [window.start_s for window in chosen]


def test_picks_three_loudest_windows_spaced_apart() -> None:
    signal = track(300, [(20, 50), (120, 150), (230, 260)])

    chosen = windows.select_windows(signal, RATE)

    assert starts(chosen) == [20.0, 120.0, 230.0]
    assert all(window.length_s == 30.0 for window in chosen)


def test_quiet_intro_and_outro_are_skipped() -> None:
    signal = track(240, [(40, 200)])

    chosen = windows.select_windows(signal, RATE)

    assert len(chosen) == 3
    assert all(window.start_s >= 40.0 and window.start_s + 30.0 <= 200.0 for window in chosen)
    gaps = np.diff(starts(chosen))
    assert np.all(gaps >= 30.0)


def test_short_track_is_one_window() -> None:
    chosen = windows.select_windows(track(12, []), RATE)

    assert chosen == [windows.Window(0.0, 12.0)]


def test_track_under_ninety_seconds_gets_overlapping_windows() -> None:
    signal = track(60, [(0, 60)])

    chosen = windows.select_windows(signal, RATE)

    assert len(chosen) == 3
    assert starts(chosen) == sorted(set(starts(chosen)))
    assert max(starts(chosen)) <= 30.0


def test_last_window_ends_at_track_end() -> None:
    signal = track(95, [(70, 95)])

    chosen = windows.select_windows(signal, RATE)

    assert 65.0 in starts(chosen)


def test_cut_stacks_equal_windows() -> None:
    signal = np.arange(90 * RATE, dtype=np.float32)
    chosen = [windows.Window(0.0, 30.0), windows.Window(60.0, 30.0)]

    clips = windows.cut(signal, RATE, chosen)

    assert clips.shape == (2, 30 * RATE)
    assert clips.dtype == np.float32
    assert clips[1, 0] == pytest.approx(60 * RATE)


def test_cut_short_track_keeps_whole_signal() -> None:
    signal = np.ones(7 * RATE, dtype=np.float32)

    clips = windows.cut(signal, RATE, windows.select_windows(signal, RATE))

    assert clips.shape == (1, 7 * RATE)
