from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from worker.domain.ports import Float32Array

WINDOW_S = 30.0
HOP_S = 10.0
WINDOW_COUNT = 3
MIN_GAP_S = 30.0
FRAME_S = 0.5


@dataclass(frozen=True)
class Window:
    start_s: float
    length_s: float


def select_windows(
    signal: Float32Array,
    sample_rate: int,
    *,
    length_s: float = WINDOW_S,
    hop_s: float = HOP_S,
    count: int = WINDOW_COUNT,
    min_gap_s: float = MIN_GAP_S,
) -> list[Window]:
    duration_s = signal.shape[0] / sample_rate
    if duration_s <= length_s:
        return [Window(0.0, duration_s)]
    starts = candidate_starts(duration_s, length_s, hop_s)
    loudness = frame_rms(signal, sample_rate)
    keys = [loudest_first(loudness, start, length_s) for start in starts]
    ranked = sorted(range(len(starts)), key=lambda index: (keys[index], starts[index]))
    chosen = spaced(ranked, starts, count, min_gap_s)
    for index in ranked:
        if len(chosen) == count:
            break
        if index not in chosen:
            chosen.append(index)
    return [Window(starts[index], length_s) for index in sorted(chosen)]


def cut(signal: Float32Array, sample_rate: int, windows: list[Window]) -> Float32Array:
    length = min(round(max(w.length_s for w in windows) * sample_rate), signal.shape[0])
    rows = []
    for window in windows:
        start = min(round(window.start_s * sample_rate), signal.shape[0] - length)
        rows.append(signal[start : start + length])
    return np.ascontiguousarray(np.stack(rows), dtype=np.float32)


def candidate_starts(duration_s: float, length_s: float, hop_s: float) -> list[float]:
    last = duration_s - length_s
    starts = [float(step * hop_s) for step in range(int(last // hop_s) + 1)]
    if last - starts[-1] > 1e-6:
        starts.append(last)
    return starts


def frame_rms(signal: Float32Array, sample_rate: int) -> Float32Array:
    frame = max(1, round(FRAME_S * sample_rate))
    frames = signal.shape[0] // frame
    blocks = signal[: frames * frame].reshape(frames, frame).astype(np.float64)
    return np.sqrt(np.mean(np.square(blocks), axis=1)).astype(np.float32)


def loudest_first(loudness: Float32Array, start_s: float, length_s: float) -> tuple[float, float]:
    first = int(start_s / FRAME_S)
    last = max(first + 1, int((start_s + length_s) / FRAME_S))
    frames = loudness[first:last]
    return -float(np.median(frames)), -float(np.mean(frames))


def spaced(ranked: list[int], starts: list[float], count: int, min_gap_s: float) -> list[int]:
    chosen: list[int] = []
    for index in ranked:
        if len(chosen) == count:
            break
        if all(abs(starts[index] - starts[other]) >= min_gap_s for other in chosen):
            chosen.append(index)
    return chosen
