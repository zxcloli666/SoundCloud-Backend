from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from itertools import pairwise

from worker.domain.lyrics.placement import LineTiming, Placement, Word, aligned_words, ordered
from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.outcome import Reason
from worker.settings import QualitySettings

INVERSION_TOLERANCE_S = 0.3
MAX_LETTERS_PER_S = 25.0
MIN_LETTERS_PER_S = 0.5
MIN_SPEECH_S = 5.0


@dataclass(frozen=True)
class Metrics:
    lines_total: int
    lines_unplaced: int
    aligned_share: float
    placed_share: float
    interpolated_share: float
    inside_share: float
    collapsed_share: float
    out_of_order_share: float
    rate_outliers: float
    anchor_agreement: float
    language_agreement: float | None
    aligner_score: float
    separated: bool

    def to_log(self) -> dict[str, object]:
        return {
            "lines_total": self.lines_total,
            "lines_unplaced": self.lines_unplaced,
            "aligned_share": round(self.aligned_share, 3),
            "placed_share": round(self.placed_share, 3),
            "interpolated_share": round(self.interpolated_share, 3),
            "inside_share": round(self.inside_share, 3),
            "collapsed_share": round(self.collapsed_share, 3),
            "out_of_order_share": round(self.out_of_order_share, 3),
            "rate_outliers": round(self.rate_outliers, 3),
            "anchor_agreement": round(self.anchor_agreement, 3),
            "language_agreement": self.language_agreement,
            "aligner_score": round(self.aligner_score, 3),
            "separated": self.separated,
        }


@dataclass(frozen=True)
class Verdict:
    metrics: Metrics
    confidence: float
    reason: Reason | None
    line_features: Mapping[int, tuple[float, ...]] = field(default_factory=dict)

    @property
    def accepted(self) -> bool:
        return self.reason is None


def empty_metrics(lines_total: int, separated: bool) -> Metrics:
    return Metrics(
        lines_total, lines_total, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, None, 0.0, separated
    )


def assess(
    placement: Placement,
    regions: Sequence[Region],
    lines: Sequence[TokenizedLine],
    *,
    lines_total: int,
    anchor_agreement: float,
    language_agreement: float | None,
    separated: bool,
    inside_tolerance_s: float,
    settings: QualitySettings,
) -> Verdict:
    metrics = measure(
        placement,
        regions,
        lines,
        lines_total=lines_total,
        anchor_agreement=anchor_agreement,
        language_agreement=language_agreement,
        separated=separated,
        inside_tolerance_s=inside_tolerance_s,
    )
    features = {
        line: tuple(line_features(timing, metrics, regions, inside_tolerance_s))
        for line, timing in placement.items()
        if timing.aligned
    }
    score = confidence(metrics, features, settings)
    return Verdict(metrics, score, gate(metrics, score, settings), features)


def measure(
    placement: Placement,
    regions: Sequence[Region],
    lines: Sequence[TokenizedLine],
    *,
    lines_total: int,
    anchor_agreement: float,
    language_agreement: float | None,
    separated: bool,
    inside_tolerance_s: float,
) -> Metrics:
    timings = ordered(placement)
    aligned = [timing for timing in timings if timing.aligned]
    words = aligned_words(placement)
    total = max(1, lines_total)
    letters = {line.ordinal: line.letters for line in lines}
    return Metrics(
        lines_total=lines_total,
        lines_unplaced=max(0, lines_total - len(timings)),
        aligned_share=len(aligned) / total,
        placed_share=len(timings) / total,
        interpolated_share=(len(timings) - len(aligned)) / total,
        inside_share=inside_share(words, regions, inside_tolerance_s),
        collapsed_share=collapsed_share(words),
        out_of_order_share=out_of_order_share([timing.start_s for timing in timings]),
        rate_outliers=rate_outliers(aligned, letters),
        anchor_agreement=anchor_agreement,
        language_agreement=language_agreement,
        aligner_score=mean([timing.region_score for timing in aligned]),
        separated=separated,
    )


def confidence(
    metrics: Metrics,
    features: Mapping[int, Sequence[float]],
    settings: QualitySettings,
) -> float:
    if settings.confidence_model == "calibrated":
        return calibrated_confidence(features, settings)
    weights = settings.confidence_v1_weights
    value = (
        weights.placed_share * metrics.placed_share
        + weights.inside_share * metrics.inside_share
        + weights.collapsed * (1.0 - metrics.collapsed_share)
        + weights.order * (1.0 - metrics.out_of_order_share)
    )
    return round(min(1.0, max(0.0, value)), 3)


def calibrated_confidence(
    features: Mapping[int, Sequence[float]], settings: QualitySettings
) -> float:
    if not features:
        return 0.0
    weights = settings.confidence_weights
    vector = (
        weights.aligned_share,
        weights.inside_share,
        weights.collapsed,
        weights.anchor,
        weights.aligner,
        weights.separated,
        weights.order,
    )
    scores = [
        sigmoid(weights.bias + sum(w * x for w, x in zip(vector, row, strict=True)))
        for row in features.values()
    ]
    return round(mean(scores), 3)


def line_features(
    timing: LineTiming,
    metrics: Metrics,
    regions: Sequence[Region],
    inside_tolerance_s: float,
) -> list[float]:
    words = list(timing.words)
    return [
        metrics.aligned_share,
        inside_share(words, regions, inside_tolerance_s),
        1.0 - collapsed_share(words),
        timing.anchor_similarity,
        timing.region_score,
        1.0 if metrics.separated else 0.0,
        1.0 - metrics.out_of_order_share,
    ]


def gate(metrics: Metrics, score: float, settings: QualitySettings) -> Reason | None:
    penalty = 0.0 if metrics.separated else settings.mix_penalty
    if (
        metrics.anchor_agreement < settings.min_anchor_agreement + penalty
        or metrics.language_agreement == 0.0
        or (
            metrics.language_agreement is None
            and metrics.anchor_agreement < settings.min_unconfirmed_anchor_agreement + penalty
        )
    ):
        return Reason.LYRICS_MISMATCH
    if metrics.out_of_order_share > settings.max_out_of_order_share - penalty:
        return Reason.OUT_OF_ORDER
    if (
        metrics.aligned_share < settings.min_aligned_share + penalty
        or metrics.placed_share < settings.min_placed_share + penalty
        or metrics.interpolated_share > settings.max_interpolated_share - penalty
    ):
        return Reason.TOO_FEW_LINES_PLACED
    if metrics.inside_share < settings.min_inside_share + penalty:
        return Reason.PLACED_IN_SILENCE
    if (
        score < settings.min_confidence + penalty
        or metrics.collapsed_share > settings.max_collapsed_share - penalty
        or metrics.rate_outliers > settings.max_rate_outliers - penalty
    ):
        return Reason.LOW_CONFIDENCE
    return None


def words_publishable(metrics: Metrics, settings: QualitySettings) -> bool:
    return metrics.collapsed_share <= settings.max_words_collapsed_share


def inside_share(words: Sequence[Word], regions: Sequence[Region], tolerance_s: float) -> float:
    if not words:
        return 0.0
    inside = sum(
        1 for word in words if any(region.contains(word.start_s, tolerance_s) for region in regions)
    )
    return inside / len(words)


def collapsed_share(words: Sequence[Word]) -> float:
    if not words:
        return 0.0
    return sum(1 for word in words if word.collapsed) / len(words)


def out_of_order_share(starts: Sequence[float]) -> float:
    if len(starts) < 2:
        return 0.0
    inverted = sum(
        1 for earlier, later in pairwise(starts) if later < earlier - INVERSION_TOLERANCE_S
    )
    return inverted / (len(starts) - 1)


def rate_outliers(timings: Sequence[LineTiming], letters: dict[int, int]) -> float:
    if not timings:
        return 0.0
    outliers = 0
    for timing in timings:
        count = letters.get(timing.line, 0)
        duration = timing.end_s - timing.start_s
        if count == 0:
            continue
        if duration <= 0:
            outliers += 1
            continue
        rate = count / duration
        if rate > MAX_LETTERS_PER_S or rate < MIN_LETTERS_PER_S:
            outliers += 1
    return outliers / len(timings)


def mean(values: Sequence[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def sigmoid(value: float) -> float:
    return 1.0 / (1.0 + math.exp(-value))
