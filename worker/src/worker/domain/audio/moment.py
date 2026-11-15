from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from worker.domain.audio import decode
from worker.domain.ports import Float32Array

VERSION = 1
ANALYSIS_RATE = 16_000
FRAME_S = 0.5
WINDOW_S = 30.0
STEP_S = 2.5
MIN_LAG_S = 20.0
INTRO_SHARE = 0.08
OUTRO_SHARE = 0.05
LOUDNESS_WEIGHT = 0.45
REPETITION_WEIGHT = 0.45
BOUNDARY_WEIGHT = 0.10
EDGE_PENALTY = 0.35
SHORT_TRACK_SCORE = 0.5
CHROMA_LOW_HZ = 55.0
CHROMA_HIGH_HZ = 5_000.0
NOVELTY_SPAN = 4
BOUNDARY_REACH = 1
ENERGY_FLOOR = 1e-10
NORM_FLOOR = 1e-9
FloatMatrix = np.ndarray[tuple[int, int], np.dtype[np.float64]]
FloatVector = np.ndarray[tuple[int], np.dtype[np.float64]]
IntVector = np.ndarray[tuple[int], np.dtype[np.int64]]


@dataclass(frozen=True)
class Moment:
    start_ms: int
    length_ms: int
    score: float


@dataclass(frozen=True)
class Frames:
    energy_db: FloatVector
    chroma: FloatMatrix


def find_moment(mono: Float32Array, sample_rate: int) -> Moment:
    signal = decode.resample(mono, sample_rate, ANALYSIS_RATE)
    duration_s = signal.shape[0] / ANALYSIS_RATE
    frames = frame_features(signal, ANALYSIS_RATE)
    window = round(WINDOW_S / FRAME_S)
    count = frames.energy_db.shape[0]
    if count <= window:
        return Moment(0, round(duration_s * 1000), SHORT_TRACK_SCORE)
    starts: IntVector = np.arange(0, count - window + 1, round(STEP_S / FRAME_S), dtype=np.int64)
    scores = window_scores(frames, starts, window, count)
    best = int(np.argmax(scores))
    start_s = float(starts[best]) * FRAME_S
    length_s = min(WINDOW_S, duration_s - start_s)
    return Moment(round(start_s * 1000), round(length_s * 1000), float(scores[best]))


def window_scores(frames: Frames, starts: IntVector, window: int, count: int) -> FloatVector:
    loudness = normalized(np.array([np.median(frames.energy_db[s : s + window]) for s in starts]))
    repetition = normalized(repetition_of(frames.chroma, starts, window))
    peaks = boundaries(frames)
    boundary = np.array(
        [float(peaks[max(0, s - BOUNDARY_REACH) : s + BOUNDARY_REACH + 1].any()) for s in starts]
    )
    blend = LOUDNESS_WEIGHT * loudness + REPETITION_WEIGHT * repetition + BOUNDARY_WEIGHT * boundary
    intro = starts < INTRO_SHARE * count
    outro = starts + window > (1.0 - OUTRO_SHARE) * count
    penalty = EDGE_PENALTY * (intro.astype(np.float64) + outro.astype(np.float64))
    scored: FloatVector = np.clip(blend - penalty, 0.0, 1.0)
    return scored


def frame_features(signal: Float32Array, rate: int) -> Frames:
    size = round(FRAME_S * rate)
    count = signal.shape[0] // size
    blocks = signal[: count * size].reshape(count, size).astype(np.float64)
    power = np.mean(np.square(blocks), axis=1)
    energy_db = 10.0 * np.log10(np.maximum(power, ENERGY_FLOOR))
    spectrum = np.square(np.abs(np.fft.rfft(blocks * np.hanning(size), axis=1)))
    frequencies = np.fft.rfftfreq(size, 1.0 / rate)
    usable = (frequencies >= CHROMA_LOW_HZ) & (frequencies <= CHROMA_HIGH_HZ)
    pitch = np.round(12.0 * np.log2(frequencies[usable] / 440.0)).astype(np.int64) + 9
    classes = np.mod(pitch, 12)
    chroma = np.zeros((count, 12))
    for pitch_class in range(12):
        chroma[:, pitch_class] = spectrum[:, usable][:, classes == pitch_class].sum(axis=1)
    norms = np.linalg.norm(chroma, axis=1, keepdims=True)
    return Frames(energy_db, chroma / np.maximum(norms, NORM_FLOOR))


def repetition_of(chroma: FloatMatrix, starts: IntVector, window: int) -> FloatVector:
    count = chroma.shape[0]
    similarity = chroma @ chroma.T
    min_lag = round(MIN_LAG_S / FRAME_S)
    best = np.zeros(starts.shape[0])
    for lag in range(min_lag, count - window + 1):
        diagonal = np.diagonal(similarity, offset=lag)
        sums = np.concatenate(([0.0], np.cumsum(diagonal)))
        means = (sums[window:] - sums[:-window]) / window
        for index, start in enumerate(starts.tolist()):
            ahead = start
            behind = start - lag
            if ahead < means.shape[0]:
                best[index] = max(best[index], means[ahead])
            if behind >= 0:
                best[index] = max(best[index], means[behind])
    return best


def boundaries(frames: Frames) -> np.ndarray[tuple[int], np.dtype[np.bool_]]:
    count = frames.energy_db.shape[0]
    novelty = np.zeros(count)
    energy = normalized(frames.energy_db)
    for frame in range(NOVELTY_SPAN, count - NOVELTY_SPAN + 1):
        before = slice(frame - NOVELTY_SPAN, frame)
        after = slice(frame, frame + NOVELTY_SPAN)
        shift = frames.chroma[after].mean(axis=0) - frames.chroma[before].mean(axis=0)
        tone = np.linalg.norm(shift)
        level = abs(energy[after].mean() - energy[before].mean())
        novelty[frame] = tone + level
    threshold = novelty.mean() + novelty.std()
    peaks = np.zeros(count, dtype=np.bool_)
    for frame in range(count):
        around = novelty[max(0, frame - NOVELTY_SPAN) : frame + NOVELTY_SPAN + 1]
        peaks[frame] = novelty[frame] > threshold and novelty[frame] >= around.max()
    return peaks


def normalized(values: FloatVector) -> FloatVector:
    low = float(values.min())
    span = float(values.max()) - low
    if span < NORM_FLOOR:
        return np.full(values.shape[0], 0.5)
    scaled: FloatVector = (values - low) / span
    return scaled
