from __future__ import annotations

from collections.abc import Sequence

from worker.domain.lyrics.placement import LineTiming, Placement, Word, runs, unplaced_lines
from worker.domain.lyrics.tokens import TokenizedLine

MAX_RUN = 2
MIN_LINE_S = 0.2


def interpolate(
    lines: Sequence[TokenizedLine], placement: Placement, *, max_gap_s: float
) -> dict[int, LineTiming]:
    added: dict[int, LineTiming] = {}
    for first, last in runs(unplaced_lines(placement, len(lines))):
        before = placement.get(first - 1)
        after = placement.get(last + 1)
        if before is None or after is None or last - first + 1 > MAX_RUN:
            continue
        gap = after.start_s - before.end_s
        if gap <= 0 or gap > max_gap_s:
            continue
        added.update(spread(lines[first : last + 1], before.end_s, after.start_s))
    return added


def spread(lines: Sequence[TokenizedLine], start_s: float, end_s: float) -> dict[int, LineTiming]:
    weights = [max(1, line.units) for line in lines]
    total = sum(weights)
    cursor = start_s
    timings: dict[int, LineTiming] = {}
    for line, weight in zip(lines, weights, strict=True):
        line_end = cursor + (end_s - start_s) * weight / total
        timings[line.ordinal] = LineTiming(
            line=line.ordinal,
            start_s=round(cursor, 3),
            end_s=round(max(line_end, cursor + min(MIN_LINE_S, end_s - cursor)), 3),
            source="interpolated",
            words=tuple(even_words(line, cursor, line_end)),
            region=None,
            region_score=0.0,
            anchor_similarity=0.0,
        )
        cursor = line_end
    return timings


def even_words(line: TokenizedLine, start_s: float, end_s: float) -> list[Word]:
    if not line.tokens:
        return []
    step = (end_s - start_s) / len(line.tokens)
    return [
        Word(
            line.ordinal,
            token,
            round(start_s + i * step, 3),
            round(start_s + (i + 1) * step, 3),
            0.0,
        )
        for i, token in enumerate(line.tokens)
    ]
