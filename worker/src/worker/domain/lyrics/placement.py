from __future__ import annotations

from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import Literal

Engine = Literal["qwen", "mms", "gapfill", "global"]
Source = Literal["qwen", "mms", "gapfill", "global", "interpolated"]

COLLAPSED_S = 0.04


@dataclass(frozen=True)
class Word:
    line: int
    text: str
    start_s: float
    end_s: float
    score: float

    @property
    def duration_s(self) -> float:
        return self.end_s - self.start_s

    @property
    def collapsed(self) -> bool:
        return self.duration_s < COLLAPSED_S


@dataclass(frozen=True)
class LineTiming:
    line: int
    start_s: float
    end_s: float
    source: Source
    words: tuple[Word, ...]
    region: int | None
    region_score: float
    anchor_similarity: float

    @property
    def aligned(self) -> bool:
        return self.source != "interpolated"


Placement = Mapping[int, LineTiming]


def timings_from_words(
    words: Iterable[Word],
    source: Source,
    region: int | None,
    region_score: float,
    anchor_similarity: float,
) -> dict[int, LineTiming]:
    grouped: dict[int, list[Word]] = {}
    for word in words:
        grouped.setdefault(word.line, []).append(word)
    timings: dict[int, LineTiming] = {}
    for line, line_words in grouped.items():
        ordered = sorted(line_words, key=lambda word: word.start_s)
        timings[line] = LineTiming(
            line=line,
            start_s=ordered[0].start_s,
            end_s=max(word.end_s for word in ordered),
            source=source,
            words=tuple(ordered),
            region=region,
            region_score=region_score,
            anchor_similarity=anchor_similarity,
        )
    return timings


def ordered(placement: Placement) -> list[LineTiming]:
    return [placement[line] for line in sorted(placement)]


def unplaced_lines(placement: Placement, total: int) -> list[int]:
    return [line for line in range(total) if line not in placement]


def aligned_words(placement: Placement) -> list[Word]:
    return [word for timing in placement.values() if timing.aligned for word in timing.words]


def runs(lines: Sequence[int]) -> list[tuple[int, int]]:
    grouped: list[tuple[int, int]] = []
    for line in lines:
        if grouped and grouped[-1][1] == line - 1:
            grouped[-1] = (grouped[-1][0], line)
        else:
            grouped.append((line, line))
    return grouped
