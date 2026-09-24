from __future__ import annotations

from dataclasses import replace

import pytest

from tests.unit.lyrics.conftest import lines_of, timing
from worker.domain.lyrics import quality
from worker.domain.lyrics.placement import LineTiming
from worker.domain.lyrics.regions import Region
from worker.domain.outcome import Reason
from worker.settings import Settings

REGIONS = [Region(0.0, 30.0), Region(40.0, 60.0)]
TEXTS = [f"строка номер {index} и ещё слова" for index in range(10)]


def clean_placement(count: int = 10) -> dict[int, LineTiming]:
    return {line: timing(line, 1.0 + line * 2.5, 1.0 + line * 2.5 + 2.0) for line in range(count)}


def assess(
    placement: dict[int, LineTiming],
    settings: Settings,
    *,
    lines_total: int = 10,
    agreement: float = 0.9,
    language_agreement: float | None = 1.0,
    separated: bool = True,
    regions: list[Region] = REGIONS,
) -> quality.Verdict:
    return quality.assess(
        placement,
        regions,
        lines_of(TEXTS),
        lines_total=lines_total,
        anchor_agreement=agreement,
        language_agreement=language_agreement,
        separated=separated,
        inside_tolerance_s=0.2,
        settings=settings.sync.quality,
    )


def test_a_clean_alignment_is_accepted(settings: Settings) -> None:
    verdict = assess(clean_placement(), settings)
    assert verdict.accepted
    assert verdict.metrics.aligned_share == 1.0
    assert verdict.metrics.placed_share == 1.0
    assert verdict.metrics.inside_share == 1.0
    assert verdict.metrics.collapsed_share == 0.0
    assert verdict.confidence == 1.0


def test_gate_order_follows_the_table(settings: Settings) -> None:
    placement = clean_placement()
    assert assess(placement, settings, agreement=0.2).reason is Reason.LYRICS_MISMATCH
    assert assess(placement, settings, language_agreement=0.0).reason is Reason.LYRICS_MISMATCH
    assert assess(placement, settings, language_agreement=None).accepted
    shuffled = dict(placement)
    shuffled[3] = timing(3, 0.5, 0.9)
    assert assess(shuffled, settings).reason is Reason.OUT_OF_ORDER
    fewer = {line: placement[line] for line in range(8)}
    assert assess(fewer, settings).reason is Reason.TOO_FEW_LINES_PLACED
    nine = {line: placement[line] for line in range(9)}
    assert assess(nine, settings).reason is Reason.TOO_FEW_LINES_PLACED
    interpolated = dict(placement)
    for line in (4, 5):
        interpolated[line] = replace(placement[line], source="interpolated")
    assert assess(interpolated, settings).reason is Reason.TOO_FEW_LINES_PLACED
    silent = {line: timing(line, 100.0 + line * 2.5, 102.0 + line * 2.5) for line in range(10)}
    assert assess(silent, settings).reason is Reason.PLACED_IN_SILENCE
    collapsed = {
        line: timing(line, 1.0 + line * 2.5, 3.0 + line * 2.5, word_s=0.0) for line in range(10)
    }
    assert assess(collapsed, settings).reason is Reason.LOW_CONFIDENCE


def test_an_unconfirmed_language_needs_a_stronger_anchor_agreement(settings: Settings) -> None:
    placement = clean_placement()
    weak = settings.sync.quality.min_unconfirmed_anchor_agreement - 0.02
    strong = settings.sync.quality.min_unconfirmed_anchor_agreement + 0.02
    assert assess(placement, settings, agreement=weak).accepted
    unconfirmed = assess(placement, settings, agreement=weak, language_agreement=None)
    assert unconfirmed.reason is Reason.LYRICS_MISMATCH
    assert assess(placement, settings, agreement=strong, language_agreement=None).accepted
    mixed = assess(placement, settings, agreement=strong, language_agreement=None, separated=False)
    assert mixed.reason is Reason.LYRICS_MISMATCH


def test_one_interpolated_line_of_ten_is_still_ok(settings: Settings) -> None:
    placement = clean_placement()
    placement[4] = replace(placement[4], source="interpolated")
    verdict = assess(placement, settings)
    assert verdict.accepted
    assert verdict.metrics.interpolated_share == 0.1
    assert verdict.metrics.aligned_share == 0.9


def test_lines_cut_by_jobs_count_as_unplaced(settings: Settings) -> None:
    verdict = assess(clean_placement(), settings, lines_total=11)
    assert verdict.metrics.lines_unplaced == 1
    assert verdict.metrics.placed_share == pytest.approx(10 / 11)
    assert verdict.reason is Reason.TOO_FEW_LINES_PLACED


def test_mix_makes_every_threshold_stricter(settings: Settings) -> None:
    placement = clean_placement()
    placement[9] = replace(placement[9], source="interpolated")
    assert assess(placement, settings, separated=True).accepted
    assert assess(placement, settings, separated=False).reason is Reason.TOO_FEW_LINES_PLACED


def test_single_inversion_is_tolerated_and_measured(settings: Settings) -> None:
    placement = clean_placement(30)
    placement[10] = timing(10, placement[9].start_s - 0.5, placement[9].start_s + 1.0)
    verdict = assess(placement, settings, lines_total=30, regions=[Region(0.0, 100.0)])
    assert verdict.metrics.out_of_order_share == pytest.approx(1 / 29)
    assert verdict.accepted


def test_rate_outliers_catch_stretched_and_squeezed_lines(settings: Settings) -> None:
    placement = clean_placement()
    placement[2] = timing(2, 6.0, 6.05)
    placement[5] = timing(5, 13.5, 100.0)
    verdict = assess(placement, settings)
    assert verdict.metrics.rate_outliers == pytest.approx(0.2)
    assert verdict.reason is Reason.LOW_CONFIDENCE


def test_v1_confidence_formula(settings: Settings) -> None:
    metrics = quality.Metrics(10, 0, 0.9, 1.0, 0.1, 0.8, 0.2, 0.0, 0.0, 0.9, 1.0, 0.9, True)
    value = quality.confidence(metrics, {}, settings.sync.quality)
    assert value == pytest.approx(0.35 * 1.0 + 0.3 * 0.8 + 0.25 * 0.8 + 0.1 * 1.0, abs=1e-3)


def test_calibrated_confidence_averages_line_sigmoids(settings: Settings) -> None:
    weights = replace(
        settings.sync.quality.confidence_weights, aligned_share=2.0, anchor=1.0, bias=-1.0
    )
    quality_settings = replace(
        settings.sync.quality, confidence_model="calibrated", confidence_weights=weights
    )
    placement = clean_placement()
    metrics = quality.measure(
        placement,
        REGIONS,
        lines_of(TEXTS),
        lines_total=10,
        anchor_agreement=0.9,
        language_agreement=1.0,
        separated=True,
        inside_tolerance_s=0.2,
    )
    features = {
        line: quality.line_features(timing, metrics, REGIONS, 0.2)
        for line, timing in placement.items()
    }
    value = quality.confidence(metrics, features, quality_settings)
    assert value == pytest.approx(quality.sigmoid(2.0 * 1.0 + 1.0 * 0.9 - 1.0), abs=1e-3)
    assert quality.confidence(metrics, {}, quality_settings) == 0.0


def test_empty_metrics_describe_a_track_with_nothing_placed() -> None:
    metrics = quality.empty_metrics(12, False)
    assert metrics.lines_total == 12 and metrics.lines_unplaced == 12
    assert metrics.placed_share == 0.0 and not metrics.separated


def test_words_are_published_only_when_not_collapsed(settings: Settings) -> None:
    metrics = quality.empty_metrics(1, True)
    assert quality.words_publishable(replace(metrics, collapsed_share=0.15), settings.sync.quality)
    assert not quality.words_publishable(
        replace(metrics, collapsed_share=0.16), settings.sync.quality
    )
