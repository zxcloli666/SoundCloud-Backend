from __future__ import annotations

from dataclasses import dataclass

import librosa
import numpy as np

from worker.domain.ports import Float32Array, Int16Array
from worker.settings import VideoSection

FINGERPRINT_RATE = 11_025
ITEM_S = 4096 / 3 / FINGERPRINT_RATE
CHROMA_HOP = 512
CHROMA_SEARCH_S = 0.6
VOTE_ROWS = 256
VOTE_TOP = 3
BITS = 32.0


@dataclass(frozen=True)
class Match:
    offset_ms: int
    peak: int
    ber: float
    overlap_s: float
    chroma: float


def int16_mono(signal: Float32Array) -> Int16Array:
    return np.asarray(np.clip(signal, -1.0, 1.0) * 32767.0, dtype=np.int16)


def parse_raw(text: str | None) -> np.ndarray | None:
    if not text:
        return None
    values = np.array([int(part) for part in text.split(",")], dtype=np.uint32)
    return values if values.size else None


def verify(
    track_raw: np.ndarray,
    video_raw: np.ndarray,
    track_mono: Float32Array,
    video_mono: Float32Array,
    settings: VideoSection,
) -> Match | None:
    best: tuple[float, int, int, float] | None = None
    for shift, peak in vote(track_raw, video_raw, settings.vote_hamming):
        if peak < settings.peak_min:
            continue
        errors = item_errors(track_raw, video_raw, shift)
        overlap_s = errors.size * ITEM_S
        if overlap_s < settings.overlap_min_s:
            continue
        ber = best_window(errors, settings.window_s, settings.window_stride_s)
        if ber < settings.ber_max and (best is None or ber < best[0]):
            best = (ber, shift, peak, overlap_s)
    if best is None:
        return None
    ber, shift, peak, overlap_s = best
    offset_ms, chroma = refine(shift, track_mono, video_mono)
    return Match(offset_ms, peak, round(ber, 4), round(overlap_s, 1), round(chroma, 4))


def vote(track: np.ndarray, video: np.ndarray, hamming: int) -> list[tuple[int, int]]:
    size = track.size + video.size
    histogram = np.zeros(size, dtype=np.int64)
    for start in range(0, track.size, VOTE_ROWS):
        rows = track[start : start + VOTE_ROWS]
        near = np.bitwise_count(rows[:, None] ^ video[None, :]) <= hamming
        row, column = np.nonzero(near)
        histogram += np.bincount(column - (row + start) + track.size, minlength=size)[:size]
    top = np.argsort(histogram)[::-1][:VOTE_TOP]
    return [(int(index) - track.size, int(histogram[index])) for index in top]


def item_errors(track: np.ndarray, video: np.ndarray, shift: int) -> np.ndarray:
    first = max(0, -shift)
    last = min(track.size, video.size - shift)
    if last <= first:
        return np.zeros(0, dtype=np.float64)
    pairs = track[first:last] ^ video[first + shift : last + shift]
    return np.asarray(np.bitwise_count(pairs), dtype=np.float64) / BITS


def best_window(errors: np.ndarray, window_s: float, stride_s: float) -> float:
    width = min(errors.size, max(1, round(window_s / ITEM_S)))
    stride = max(1, round(stride_s / ITEM_S))
    starts = range(0, errors.size - width + 1, stride)
    return min(float(errors[start : start + width].mean()) for start in starts)


def chroma_of(signal: Float32Array) -> np.ndarray:
    chroma = librosa.feature.chroma_stft(y=signal, sr=FINGERPRINT_RATE, hop_length=CHROMA_HOP)
    norm = np.linalg.norm(chroma, axis=0, keepdims=True)
    return np.asarray(chroma / np.maximum(norm, 1e-9), dtype=np.float32)


def refine(shift: int, track_mono: Float32Array, video_mono: Float32Array) -> tuple[int, float]:
    hop_s = CHROMA_HOP / FINGERPRINT_RATE
    track = chroma_of(track_mono)
    video = chroma_of(video_mono)
    centre = round(shift * ITEM_S / hop_s)
    span = round(CHROMA_SEARCH_S / hop_s)
    best_lag, best_score = centre, -1.0
    for lag in range(centre - span, centre + span + 1):
        first = max(0, -lag)
        last = min(track.shape[1], video.shape[1] - lag)
        if last - first < 1:
            continue
        score = float(
            (track[:, first:last] * video[:, first + lag : last + lag]).sum(axis=0).mean()
        )
        if score > best_score:
            best_lag, best_score = lag, score
    return round(best_lag * hop_s * 1000.0), best_score
