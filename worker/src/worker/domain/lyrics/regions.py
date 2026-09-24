from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

import numpy as np

from worker.domain.ports import Float32Array, Span

SAMPLE_RATE = 16_000
ENERGY_FRAME_S = 0.05
SPLIT_SEARCH_SHARE = 0.25
MIN_CLIP_S = 0.1


@dataclass(frozen=True)
class Region:
    start_s: float
    end_s: float

    @property
    def duration_s(self) -> float:
        return self.end_s - self.start_s

    def contains(self, moment_s: float, tolerance_s: float = 0.0) -> bool:
        return self.start_s - tolerance_s <= moment_s <= self.end_s + tolerance_s


def build(
    spans: Sequence[Span], vocals: Float32Array, *, min_s: float, max_s: float
) -> list[Region]:
    duration_s = vocals.shape[0] / SAMPLE_RATE
    merged = merge(spans, duration_s)
    regions: list[Region] = []
    for region in merged:
        regions.extend(split_long(region, vocals, max_s))
    return [region for region in regions if region.duration_s >= min_s]


def total_speech_s(regions: Sequence[Region]) -> float:
    return sum(region.duration_s for region in regions)


def clip(vocals: Float32Array, start_s: float, end_s: float) -> tuple[Float32Array, float]:
    total = vocals.shape[0]
    first = max(0, min(total, round(start_s * SAMPLE_RATE)))
    last = max(first, min(total, round(end_s * SAMPLE_RATE)))
    return np.ascontiguousarray(vocals[first:last]), first / SAMPLE_RATE


def usable(samples: Float32Array) -> bool:
    return int(samples.shape[0]) >= MIN_CLIP_S * SAMPLE_RATE


def merge(spans: Sequence[Span], duration_s: float) -> list[Region]:
    ordered = sorted((max(0.0, span.start_s), min(duration_s, span.end_s)) for span in spans)
    merged: list[Region] = []
    for start, end in ordered:
        if end <= start:
            continue
        if merged and start <= merged[-1].end_s:
            merged[-1] = Region(merged[-1].start_s, max(merged[-1].end_s, end))
        else:
            merged.append(Region(start, end))
    return merged


def split_long(region: Region, vocals: Float32Array, max_s: float) -> list[Region]:
    if region.duration_s <= max_s:
        return [region]
    cut_s = quietest_moment(vocals, region, max_s)
    head = Region(region.start_s, cut_s)
    return [head, *split_long(Region(cut_s, region.end_s), vocals, max_s)]


def quietest_moment(vocals: Float32Array, region: Region, max_s: float) -> float:
    target = region.start_s + max_s
    window = max_s * SPLIT_SEARCH_SHARE
    low = max(region.start_s + max_s * SPLIT_SEARCH_SHARE, target - window)
    high = min(region.end_s - max_s * SPLIT_SEARCH_SHARE, target)
    if high <= low:
        return target
    frame = round(ENERGY_FRAME_S * SAMPLE_RATE)
    first = round(low * SAMPLE_RATE)
    last = round(high * SAMPLE_RATE)
    frames = (last - first) // frame
    if frames < 1:
        return target
    block = vocals[first : first + frames * frame].reshape(frames, frame).astype(np.float64)
    energy = np.mean(np.square(block), axis=1)
    quietest = int(np.argmin(energy))
    return (first + quietest * frame + frame / 2) / SAMPLE_RATE
