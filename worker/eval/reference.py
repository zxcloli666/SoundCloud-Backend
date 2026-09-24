from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from itertools import pairwise

from eval import stats

INLIER_S = 0.5
MIN_LINES = 4
MIN_INLIER_SHARE = 0.5
MIN_GAIN_SHARE = 0.1
TEMPO_SCALES = tuple(1.0 + step * 0.0025 for step in range(-20, 21))
COARSE_FRACTION_S = 0.1
COARSE_SHARE = 0.8
DENSE_SHARE = 0.1
EDIT_RUN_LINES = 5
EDIT_OFFSET_S = 1.0

Pairs = Mapping[int, tuple[float, float]]


@dataclass(frozen=True)
class Fit:
    scale: float
    shift_s: float
    inlier_share: float
    applied: bool

    def expected(self, reference_s: float) -> float:
        if not self.applied:
            return reference_s
        return self.scale * reference_s + self.shift_s


@dataclass(frozen=True)
class ReferenceLine:
    start_s: float
    shortest_s: float


@dataclass(frozen=True)
class ReferenceCheck:
    fit: Fit
    flags: tuple[str, ...]

    @property
    def usable(self) -> bool:
        return not self.flags

    def deltas(self, pairs: Pairs) -> dict[int, float]:
        return {
            line: produced - self.fit.expected(reference)
            for line, (produced, reference) in pairs.items()
        }


IDENTITY = Fit(1.0, 0.0, 0.0, False)


def check(pairs: Pairs, lines: Sequence[ReferenceLine]) -> ReferenceCheck:
    fitted = fit(pairs)
    residuals = [
        produced - fitted.expected(reference)
        for produced, reference in sorted(pairs.values(), key=lambda pair: pair[1])
    ]
    raised = (
        ("coarse", coarse([line.start_s for line in lines])),
        ("dense", dense(lines)),
        ("edited", edited(residuals)),
    )
    return ReferenceCheck(fitted, tuple(name for name, flagged in raised if flagged))


def fit(pairs: Pairs) -> Fit:
    points = list(pairs.values())
    if len(points) < MIN_LINES:
        return IDENTITY
    needed = max(1, math.ceil(MIN_GAIN_SHARE * len(points)))
    identity = Fit(1.0, 0.0, round(len(inliers(points, 1.0, 0.0)) / len(points), 4), False)
    chosen = best_fit(points, (1.0,))
    drifting = best_fit(points, TEMPO_SCALES)
    if gained(drifting, chosen, points) >= needed:
        chosen = drifting
    if chosen.inlier_share < MIN_INLIER_SHARE or gained(chosen, identity, points) < needed:
        return identity
    return chosen


def best_fit(points: Sequence[tuple[float, float]], scales: Sequence[float]) -> Fit:
    best = (1.0, 0.0)
    best_rank = rank(points, *best)
    for scale in scales:
        for produced, reference in points:
            candidate = (scale, produced - scale * reference)
            candidate_rank = rank(points, *candidate)
            if candidate_rank > best_rank:
                best, best_rank = candidate, candidate_rank
    scale = best[0]
    hits = inliers(points, *best)
    shift = stats.median([points[index][0] - scale * points[index][1] for index in hits])
    share = len(inliers(points, scale, shift)) / len(points)
    return Fit(round(scale, 4), round(shift, 3), round(share, 4), True)


def rank(points: Sequence[tuple[float, float]], scale: float, shift: float) -> tuple[int, float]:
    hits = inliers(points, scale, shift)
    spread = sum(abs(points[index][0] - scale * points[index][1] - shift) for index in hits)
    return len(hits), -spread


def gained(candidate: Fit, base: Fit, points: Sequence[tuple[float, float]]) -> int:
    return round((candidate.inlier_share - base.inlier_share) * len(points))


def inliers(points: Sequence[tuple[float, float]], scale: float, shift: float) -> list[int]:
    return [
        index
        for index, (produced, reference) in enumerate(points)
        if abs(produced - scale * reference - shift) <= INLIER_S
    ]


def coarse(reference_times: Sequence[float]) -> bool:
    if len(reference_times) < MIN_LINES:
        return False
    whole = sum(1 for moment in reference_times if moment % 1.0 < COARSE_FRACTION_S)
    return whole / len(reference_times) >= COARSE_SHARE


def dense(lines: Sequence[ReferenceLine]) -> bool:
    if len(lines) < MIN_LINES:
        return False
    rushed = sum(
        1
        for current, following in pairwise(lines)
        if following.start_s - current.start_s < current.shortest_s
    )
    return rushed / len(lines) >= DENSE_SHARE


def edited(residuals: Sequence[float]) -> bool:
    start = 0
    while start < len(residuals):
        end = start + 1
        while end < len(residuals) and tight(residuals[start : end + 1]):
            end += 1
        run = residuals[start:end]
        if len(run) >= EDIT_RUN_LINES and abs(stats.median(run)) >= EDIT_OFFSET_S:
            return True
        start = end
    return False


def tight(run: Sequence[float]) -> bool:
    centre = stats.median(run)
    return all(abs(value - centre) <= INLIER_S for value in run)
