from __future__ import annotations

import math
import re
from collections.abc import Mapping, Sequence
from dataclasses import asdict, dataclass
from difflib import SequenceMatcher
from functools import cached_property
from typing import Any

from eval import reference, stats
from eval.lyricsfile import RefWord, normalize

THRESHOLDS_MS = (50, 100, 200, 300)
MAX_FIT_POINTS = 80
HYPHENS = re.compile(r"[-‐-―]")


@dataclass(frozen=True)
class Piece:
    key: str
    start_s: float
    interpolated: bool
    unplaced: bool


@dataclass(frozen=True)
class WordPair:
    produced_s: float
    reference_s: float
    interpolated: bool


@dataclass(frozen=True)
class WordResult:
    ref_words: int
    produced_words: int
    unplaced: int
    withheld: bool
    pairs: tuple[WordPair, ...]

    @cached_property
    def fit(self) -> reference.Fit:
        stride = max(1, math.ceil(len(self.pairs) / MAX_FIT_POINTS))
        sample = self.pairs[::stride]
        return reference.fit(
            {index: (pair.produced_s, pair.reference_s) for index, pair in enumerate(sample)}
        )

    def deltas_ms(self, *, fitted: bool, interpolated: bool | None) -> list[float]:
        return [
            (pair.produced_s - self.expected_s(pair, fitted)) * 1000.0
            for pair in self.pairs
            if interpolated is None or pair.interpolated == interpolated
        ]

    def expected_s(self, pair: WordPair, fitted: bool) -> float:
        return self.fit.expected(pair.reference_s) if fitted else pair.reference_s


def score(
    reference_words: Sequence[RefWord],
    produced_words: Sequence[Mapping[str, Any]],
    withheld: bool,
    per_character: bool,
) -> WordResult:
    expected = reference_pieces(reference_words, per_character)
    produced = produced_pieces(produced_words, per_character)
    matcher = SequenceMatcher(
        None, [piece.key for piece in produced], [piece.key for piece in expected], autojunk=False
    )
    return WordResult(
        ref_words=len(expected),
        produced_words=len(produced),
        unplaced=sum(1 for piece in produced if piece.unplaced),
        withheld=withheld,
        pairs=tuple(pairs_of(matcher, produced, expected)),
    )


def pairs_of(
    matcher: SequenceMatcher[str], produced: Sequence[Piece], expected: Sequence[Piece]
) -> list[WordPair]:
    pairs: list[WordPair] = []
    for tag, i1, i2, j1, _ in matcher.get_opcodes():
        if tag != "equal":
            continue
        for offset in range(i2 - i1):
            ours = produced[i1 + offset]
            if not ours.unplaced:
                pairs.append(
                    WordPair(ours.start_s, expected[j1 + offset].start_s, ours.interpolated)
                )
    return pairs


def reference_pieces(words: Sequence[RefWord], per_character: bool) -> list[Piece]:
    return [
        Piece(key, moment / 1000.0, False, False)
        for word in words
        for key, moment in spread(word.text, word.start_ms, word.end_ms, per_character)
    ]


def produced_pieces(words: Sequence[Mapping[str, Any]], per_character: bool) -> list[Piece]:
    return [
        Piece(key, moment / 1000.0, bool(word["interpolated"]), bool(word["unplaced"]))
        for word in words
        for key, moment in spread(
            str(word["text"]), int(word["start_ms"]), int(word["end_ms"]), per_character
        )
    ]


def spread(text: str, start_ms: int, end_ms: int, per_character: bool) -> list[tuple[str, float]]:
    parts = [normalize(part) for part in HYPHENS.split(text)]
    keys = [piece for part in parts for piece in (list(part) if per_character else [part]) if piece]
    total = sum(len(key) for key in keys)
    spread_keys: list[tuple[str, float]] = []
    passed = 0
    for key in keys:
        spread_keys.append((key, start_ms + (end_ms - start_ms) * passed / total))
        passed += len(key)
    return spread_keys


def metrics(results: Sequence[WordResult]) -> dict[str, object]:
    reference_total = sum(result.ref_words for result in results)
    paired = sum(len(result.pairs) for result in results)
    return {
        "tracks": len(results),
        "tracks_with_words": sum(1 for result in results if result.produced_words),
        "withheld_by_shipped_gate": sum(1 for result in results if result.withheld),
        "fit_applied": sum(1 for result in results if result.fit.applied),
        "ref_words": reference_total,
        "produced_words": sum(result.produced_words for result in results),
        "paired": paired,
        "unplaced": sum(result.unplaced for result in results),
        "coverage": round(paired / reference_total, 4) if reference_total else None,
        "all": flavors(results, None),
        "aligned": flavors(results, False),
        "interpolated": flavors(results, True),
    }


def track_metrics(result: WordResult) -> dict[str, object]:
    fit = result.fit
    return {
        **metrics([result]),
        "fit": {
            "scale": fit.scale,
            "shift_s": fit.shift_s,
            "inlier_share": fit.inlier_share,
            "applied": fit.applied,
        },
    }


def flavors(results: Sequence[WordResult], interpolated: bool | None) -> dict[str, object]:
    return {
        "raw": timing(
            [d for r in results for d in r.deltas_ms(fitted=False, interpolated=interpolated)]
        ),
        "fitted": timing(
            [d for r in results for d in r.deltas_ms(fitted=True, interpolated=interpolated)]
        ),
    }


def timing(deltas_ms: Sequence[float]) -> dict[str, object]:
    if not deltas_ms:
        return {
            "n": 0,
            "median_abs_delta_ms": None,
            "bias_ms": None,
            **{f"acc@{t}ms": None for t in THRESHOLDS_MS},
        }
    return {
        "n": len(deltas_ms),
        "median_abs_delta_ms": round(stats.median([abs(d) for d in deltas_ms]), 1),
        "bias_ms": round(stats.median(deltas_ms), 1),
        **{f"acc@{t}ms": round(stats.accuracy_at(deltas_ms, t), 4) for t in THRESHOLDS_MS},
    }


def to_json(result: WordResult) -> dict[str, object]:
    return asdict(result)


def from_json(raw: Mapping[str, Any] | None) -> WordResult | None:
    if raw is None:
        return None
    return WordResult(
        ref_words=int(raw["ref_words"]),
        produced_words=int(raw["produced_words"]),
        unplaced=int(raw["unplaced"]),
        withheld=bool(raw["withheld"]),
        pairs=tuple(
            WordPair(float(p["produced_s"]), float(p["reference_s"]), bool(p["interpolated"]))
            for p in raw["pairs"]
        ),
    )
