from __future__ import annotations

from collections.abc import Sequence

from worker.domain.deadline import Deadline
from worker.domain.lyrics import regions as region_tools
from worker.domain.lyrics.align import ctc_align
from worker.domain.lyrics.placement import (
    LineTiming,
    Placement,
    Word,
    runs,
    timings_from_words,
    unplaced_lines,
)
from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.ports import Engines, Float32Array
from worker.observability.counters import Counters


async def fill(
    engines: Engines,
    vocals: Float32Array,
    regions: Sequence[Region],
    lines: Sequence[TokenizedLine],
    placement: Placement,
    *,
    max_gap_s: float,
    counters: Counters,
    deadline: Deadline,
) -> dict[int, LineTiming]:
    track_end = vocals.shape[0] / region_tools.SAMPLE_RATE
    added: dict[int, LineTiming] = {}
    for first, last in runs(unplaced_lines(placement, len(lines))):
        before = placement.get(first - 1)
        after = placement.get(last + 1)
        start = before.end_s if before is not None else 0.0
        end = after.start_s if after is not None else track_end
        if end - start <= 0 or end - start > max_gap_s or not has_speech(regions, start, end):
            continue
        deadline.check("gap_fill")
        samples, offset = region_tools.clip(vocals, start, end)
        if not region_tools.usable(samples):
            continue
        tokens, alignment = await ctc_align(
            engines, samples, lines[first : last + 1], counters, "gap_fill", deadline
        )
        if not tokens:
            continue
        counters.inc("align_engine_total", engine="gapfill")
        words = [
            Word(
                line,
                text,
                round(offset + span.start_s, 3),
                round(offset + span.end_s, 3),
                span.score,
            )
            for (line, text), span in zip(tokens, alignment.spans, strict=True)
        ]
        added.update(timings_from_words(words, "gapfill", None, alignment.score, 0.0))
    return added


def has_speech(regions: Sequence[Region], start_s: float, end_s: float) -> bool:
    return any(region.start_s < end_s and region.end_s > start_s for region in regions)
