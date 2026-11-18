from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from worker.domain.video.features import Features
from worker.settings import VideoClassifierSettings

Kind = Literal["clip", "loop", "static"]


@dataclass(frozen=True)
class Verdict:
    kind: Kind
    uncertain: bool
    rule: str


def classify(features: Features, settings: VideoClassifierSettings) -> Verdict:
    cuts_ok = features.cuts_per_min < settings.cut_limit_per_min
    statics = [
        (
            "still",
            features.motion_med < settings.still_motion_med
            and features.motion_p90 < settings.still_motion_p90,
        ),
        ("zoom", features.global_explained >= settings.zoom_explained and cuts_ok),
        ("over_still", features.static_share >= settings.over_still_static_share and cuts_ok),
        ("animated_cover", features.clip_div < settings.animated_clip_div and cuts_ok),
    ]
    for rule, hit in statics:
        if hit:
            return Verdict("static", close_to_static(features, settings), rule)
    close = close_to_static(features, settings)
    if features.duration_s <= settings.short_file_s:
        return Verdict("loop", close, "short_file")
    if features.loop_score >= settings.loop_certain:
        return Verdict(
            "loop",
            close or near(features.loop_score, settings.loop_certain, settings),
            "loop_certain",
        )
    if features.loop_score >= settings.loop_likely and cuts_ok:
        return Verdict("loop", True, "loop_likely")
    uncertain = close or near(features.loop_score, settings.loop_likely, settings)
    return Verdict("clip", uncertain, "clip")


def close_to_static(features: Features, settings: VideoClassifierSettings) -> bool:
    margin = settings.static_margin
    thresholds = (
        (features.motion_med, settings.still_motion_med),
        (features.motion_p90, settings.still_motion_p90),
        (features.global_explained, settings.zoom_explained),
        (features.static_share, settings.over_still_static_share),
        (features.clip_div, settings.animated_clip_div),
    )
    if any(relative(value, limit) < margin for value, limit in thresholds):
        return True
    return relative(features.cuts_per_min, settings.cut_limit_per_min) < settings.uncertain_margin


def near(value: float, limit: float, settings: VideoClassifierSettings) -> bool:
    return relative(value, limit) < settings.uncertain_margin


def relative(value: float, limit: float) -> float:
    return abs(value - limit) / limit
