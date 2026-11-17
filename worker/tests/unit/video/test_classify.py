from __future__ import annotations

import dataclasses

import pytest

from worker.domain.video.classify import classify
from worker.domain.video.features import Features
from worker.settings import Settings

CLIP = Features(
    duration_s=210.0,
    motion_med=0.78,
    motion_p90=0.88,
    static_share=0.05,
    quiet_time=0.0,
    global_explained=-0.01,
    cuts_per_min=17.0,
    loop_score=0.1,
    loop_period_s=30.0,
    clip_div=0.38,
)


def with_(**changes: float) -> Features:
    return dataclasses.replace(CLIP, **changes)


@pytest.mark.parametrize(
    ("features", "kind", "rule"),
    [
        (CLIP, "clip", "clip"),
        (with_(motion_med=0.0, motion_p90=0.01, cuts_per_min=0.0), "static", "still"),
        (with_(global_explained=0.97, cuts_per_min=0.0, motion_med=0.3), "static", "zoom"),
        (with_(static_share=0.89, cuts_per_min=0.0, motion_med=0.1), "static", "over_still"),
        (with_(clip_div=0.01, cuts_per_min=0.0), "static", "animated_cover"),
        (with_(duration_s=8.0, cuts_per_min=0.0), "loop", "short_file"),
        (with_(loop_score=1.0, loop_period_s=5.0), "loop", "loop_certain"),
        (with_(loop_score=0.9, cuts_per_min=0.0), "loop", "loop_likely"),
    ],
)
def test_the_rule_picks_the_class(
    settings: Settings, features: Features, kind: str, rule: str
) -> None:
    verdict = classify(features, settings.video.classifier)
    assert (verdict.kind, verdict.rule) == (kind, rule)


def test_a_clear_clip_is_certain(settings: Settings) -> None:
    assert not classify(CLIP, settings.video.classifier).uncertain


def test_a_clip_close_to_a_static_threshold_is_uncertain(settings: Settings) -> None:
    limit = settings.video.classifier.animated_clip_div
    verdict = classify(with_(clip_div=limit * 1.1), settings.video.classifier)
    assert (verdict.kind, verdict.uncertain) == ("clip", True)


def test_a_loop_of_a_cut_heavy_clip_is_not_a_clip(settings: Settings) -> None:
    verdict = classify(with_(loop_score=0.99, cuts_per_min=47.0), settings.video.classifier)
    assert verdict.kind == "loop"
