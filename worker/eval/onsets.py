from __future__ import annotations

import asyncio
import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import librosa
import numpy as np

from eval import reference
from eval.engines import LocalEngines
from eval.words import WordResult
from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.lyrics.transcribe import SPEECH_SAMPLE_RATE, separation_mix, speech_mono
from worker.domain.outcome import PermanentFailure, TransientFailure
from worker.domain.ports import EngineUnavailable, Float32Array
from worker.settings import AudioSection

HOP = SPEECH_SAMPLE_RATE // 100
HOP_S = HOP / SPEECH_SAMPLE_RATE
NORMALIZE_PERCENTILE = 95.0
WINDOW_BEFORE_S = 0.02
WINDOW_AFTER_S = 0.04
SHIFTS_MS = tuple(range(-200, 201, 10))
CONTROL_MS = 100
CONTROL_STEPS = CONTROL_MS // (SHIFTS_MS[1] - SHIFTS_MS[0])
NEAR_MS = (30, 50)
TIE_MS = 10.0
DISAGREE_MS = 80.0
TIMELINES = ("ours", "lrclib")
INVALID_NOTE = (
    "best-shift timelines do not beat their controls 100 ms either side, the judge is meaningless"
)

log = logging.getLogger("eval.onsets")

Array = np.ndarray[Any, np.dtype[np.float64]]


@dataclass(frozen=True)
class OnsetResult:
    pairs: tuple[tuple[float, float, bool], ...]
    onsets_s: tuple[float, ...]
    fit: reference.Fit
    sweep: Mapping[str, tuple[float, ...]]

    def timeline(self, name: str, shift_ms: int = 0) -> Array:
        column = 0 if name == "ours" else 1
        values = np.array([pair[column] for pair in self.pairs], dtype=np.float64)
        return values + shift_ms / 1000.0

    def distances_ms(self, name: str, shift_ms: int = 0) -> Array:
        return nearest_distances_ms(self.timeline(name, shift_ms), np.array(self.onsets_s))


@dataclass(frozen=True)
class Curve:
    distances_ms: tuple[Array, ...]
    strength: tuple[float, ...]

    def median_ms(self, index: int) -> float:
        return float(np.median(self.distances_ms[index]))

    def medians_ms(self) -> list[float]:
        return [self.median_ms(index) for index in range(len(SHIFTS_MS))]


async def judge(
    engines: LocalEngines,
    audio_settings: AudioSection,
    audio_path: Path,
    words: WordResult,
    deadline: Deadline,
) -> OnsetResult | None:
    if not words.pairs:
        return None
    vocals = await vocals_16k(engines, audio_settings, audio_path, deadline)
    if vocals is None:
        return None
    envelope = onset_envelope(vocals)
    fit = words.fit
    pairs = tuple(
        (round(pair.produced_s, 3), round(fit.expected(pair.reference_s), 3), pair.interpolated)
        for pair in words.pairs
    )
    onsets = tuple(round(float(t), 3) for t in onset_times(envelope))
    partial = OnsetResult(pairs, onsets, fit, {})
    sweep = {
        name: tuple(
            round(float(np.mean(window_strengths(partial.timeline(name, shift), envelope))), 4)
            for shift in SHIFTS_MS
        )
        for name in TIMELINES
    }
    return OnsetResult(pairs, onsets, fit, sweep)


async def vocals_16k(
    engines: LocalEngines, audio_settings: AudioSection, audio_path: Path, deadline: Deadline
) -> Float32Array | None:
    pcm = await decode.decode(
        audio_path, max_duration_s=audio_settings.max_duration_s, deadline=deadline
    )
    mix = await asyncio.to_thread(separation_mix, pcm)
    try:
        separation = await engines.separate(mix, deadline, budget=deadline)
    except (EngineUnavailable, PermanentFailure, TransientFailure) as error:
        log.warning("no vocals stem for %s: %r", audio_path.name, error)
        return None
    if separation.vocals.shape != mix.shape or not np.all(np.isfinite(separation.vocals)):
        log.warning("invalid vocals stem for %s", audio_path.name)
        return None
    return await asyncio.to_thread(speech_mono, separation.vocals)


def onset_envelope(vocals_mono_16k: Float32Array) -> Array:
    envelope = librosa.onset.onset_strength(
        y=vocals_mono_16k, sr=SPEECH_SAMPLE_RATE, hop_length=HOP
    ).astype(np.float64)
    scale = float(np.percentile(envelope, NORMALIZE_PERCENTILE)) if envelope.size else 0.0
    return envelope / scale if scale > 0 else envelope


def onset_times(envelope: Array) -> Array:
    if envelope.size == 0:
        return np.zeros(0)
    return np.asarray(
        librosa.onset.onset_detect(
            onset_envelope=envelope, sr=SPEECH_SAMPLE_RATE, hop_length=HOP, units="time"
        ),
        dtype=np.float64,
    )


def nearest_distances_ms(times_s: Array, onsets_s: Array) -> Array:
    if onsets_s.size == 0:
        return np.full(times_s.shape, np.inf)
    ordered = np.sort(onsets_s)
    right = np.clip(np.searchsorted(ordered, times_s), 0, ordered.size - 1)
    left = np.clip(right - 1, 0, ordered.size - 1)
    nearest = np.minimum(np.abs(ordered[right] - times_s), np.abs(ordered[left] - times_s))
    return nearest * 1000.0


def window_strengths(times_s: Array, envelope: Array) -> Array:
    if envelope.size == 0:
        return np.zeros(times_s.shape)
    first = np.clip(np.floor((times_s - WINDOW_BEFORE_S) / HOP_S).astype(int), 0, envelope.size - 1)
    last = np.clip(np.ceil((times_s + WINDOW_AFTER_S) / HOP_S).astype(int), 0, envelope.size - 1)
    return np.array(
        [envelope[low : high + 1].max() for low, high in zip(first, last, strict=True)],
        dtype=np.float64,
    )


def scores(distances_ms: Array, strength: float) -> dict[str, object]:
    if distances_ms.size == 0:
        return {"n": 0}
    return {
        "n": int(distances_ms.size),
        "median_ms": round(float(np.median(distances_ms)), 1),
        **{f"within_{t}ms": round(float(np.mean(distances_ms <= t)), 4) for t in NEAR_MS},
        "strength": round(strength, 4),
    }


def discriminates(real: Mapping[str, Any], controls: Sequence[Mapping[str, Any]]) -> bool:
    if not real.get("n"):
        return False
    return all(
        real["median_ms"] < control["median_ms"] and real["strength"] > control["strength"]
        for control in controls
        if control.get("n")
    )


def head_to_head(
    ours_ms: Array, lrclib_ms: Array, ours_s: Array, lrclib_s: Array
) -> dict[str, object]:
    disagree = np.abs(ours_s - lrclib_s) * 1000.0 >= DISAGREE_MS
    gain = lrclib_ms[disagree] - ours_ms[disagree]
    return {
        "all": wins(ours_ms, lrclib_ms),
        "disagree_80ms": {
            **wins(ours_ms[disagree], lrclib_ms[disagree]),
            "median_ours_gain_ms": round(float(np.median(gain)), 1) if gain.size else None,
        },
    }


def wins(ours_ms: Array, lrclib_ms: Array) -> dict[str, object]:
    total = ours_ms.size
    if total == 0:
        return {"n": 0, "ours": None, "lrclib": None, "tie": None}
    gap = lrclib_ms - ours_ms
    return {
        "n": total,
        "ours": round(float(np.mean(gap > TIE_MS)), 4),
        "lrclib": round(float(np.mean(gap < -TIE_MS)), 4),
        "tie": round(float(np.mean(np.abs(gap) <= TIE_MS)), 4),
    }


def pooled_curve(results: Sequence[OnsetResult], name: str) -> Curve:
    weights = np.array([len(r.pairs) for r in results], dtype=np.float64)
    return Curve(
        tuple(
            np.concatenate([r.distances_ms(name, shift) for r in results]) for shift in SHIFTS_MS
        ),
        tuple(
            float(np.average([r.sweep[name][index] for r in results], weights=weights))
            for index in range(len(SHIFTS_MS))
        ),
    )


def best_index(values: Sequence[float], *, maximize: bool) -> int:
    sign = -1.0 if maximize else 1.0
    return min(
        range(len(SHIFTS_MS)),
        key=lambda index: (sign * round(values[index], 4), abs(SHIFTS_MS[index])),
    )


def best_shifts(curve: Curve) -> dict[str, int]:
    return {
        "by_strength": SHIFTS_MS[best_index(curve.strength, maximize=True)],
        "by_median": SHIFTS_MS[best_index(curve.medians_ms(), maximize=False)],
    }


def sweep_summary(curve: Curve) -> dict[str, object]:
    zero = SHIFTS_MS.index(0)
    best = {
        "by_strength": best_index(curve.strength, maximize=True),
        "by_median": best_index(curve.medians_ms(), maximize=False),
    }
    return {
        criterion: {
            "shift_ms": SHIFTS_MS[index],
            "at_best": scores(curve.distances_ms[index], curve.strength[index]),
            "at_zero": scores(curve.distances_ms[zero], curve.strength[zero]),
        }
        for criterion, index in best.items()
    }


def best_with_controls(curve: Curve) -> dict[str, Any]:
    best = best_index(curve.strength, maximize=True)
    return {
        "shift_ms": SHIFTS_MS[best],
        "at_best": index_scores(curve, best),
        "minus": index_scores(curve, best - CONTROL_STEPS),
        "plus": index_scores(curve, best + CONTROL_STEPS),
    }


def index_scores(curve: Curve, index: int) -> dict[str, object]:
    if not 0 <= index < len(SHIFTS_MS):
        return {"n": 0}
    return scores(curve.distances_ms[index], curve.strength[index])


def shift_distribution(results: Sequence[OnsetResult]) -> dict[str, object]:
    per_track = [best_shifts(pooled_curve([result], "ours")) for result in results]
    return {
        criterion: {
            "tracks": len(per_track),
            "median": float(np.median([item[criterion] for item in per_track])),
            "q25": float(np.percentile([item[criterion] for item in per_track], 25)),
            "q75": float(np.percentile([item[criterion] for item in per_track], 75)),
        }
        for criterion in ("by_strength", "by_median")
    }


def metrics(results: Sequence[OnsetResult]) -> dict[str, object]:
    curves = {name: pooled_curve(results, name) for name in TIMELINES}
    at = {
        f"{name}{label}": scores(
            curves[name].distances_ms[SHIFTS_MS.index(shift)],
            curves[name].strength[SHIFTS_MS.index(shift)],
        )
        for name in TIMELINES
        for label, shift in (("", 0), ("/minus", -CONTROL_MS), ("/plus", CONTROL_MS))
    }
    around = {name: best_with_controls(curves[name]) for name in TIMELINES}
    verdicts = {
        name: discriminates(found["at_best"], [found["minus"], found["plus"]])
        for name, found in around.items()
    }
    valid = all(verdicts.values())
    return {
        "tracks": len(results),
        "onsets": sum(len(r.onsets_s) for r in results),
        "scores": at,
        "best_scores": around,
        "discriminates": verdicts,
        "judge_valid": valid,
        "note": None if valid else INVALID_NOTE,
        "head_to_head": head_to_head(
            np.concatenate([r.distances_ms("ours") for r in results]),
            np.concatenate([r.distances_ms("lrclib") for r in results]),
            np.concatenate([r.timeline("ours") for r in results]),
            np.concatenate([r.timeline("lrclib") for r in results]),
        ),
        "shift_sweep": {name: sweep_summary(curves[name]) for name in TIMELINES},
        "ours_best_shift_ms": shift_distribution(results),
    }


def to_json(result: OnsetResult) -> dict[str, object]:
    return {
        "pairs": [list(pair) for pair in result.pairs],
        "onsets_s": list(result.onsets_s),
        "fit": {
            "scale": result.fit.scale,
            "shift_s": result.fit.shift_s,
            "inlier_share": result.fit.inlier_share,
            "applied": result.fit.applied,
        },
        "sweep": {name: list(values) for name, values in result.sweep.items()},
    }


def from_json(raw: Mapping[str, Any] | None) -> OnsetResult | None:
    if raw is None:
        return None
    fit = raw["fit"]
    return OnsetResult(
        pairs=tuple((float(a), float(b), bool(c)) for a, b, c in raw["pairs"]),
        onsets_s=tuple(float(t) for t in raw["onsets_s"]),
        fit=reference.Fit(
            float(fit["scale"]),
            float(fit["shift_s"]),
            float(fit["inlier_share"]),
            bool(fit["applied"]),
        ),
        sweep={name: tuple(float(v) for v in values) for name, values in raw["sweep"].items()},
    )


def track_summary(result: OnsetResult) -> dict[str, object]:
    curves = {name: pooled_curve([result], name) for name in TIMELINES}
    return {
        **metrics([result]),
        **to_json(result),
        "shifts_ms": list(SHIFTS_MS),
        "median_curve_ms": {
            name: [round(value, 1) for value in curve.medians_ms()]
            for name, curve in curves.items()
        },
        "best_shift_ms": best_shifts(curves["ours"]),
    }
