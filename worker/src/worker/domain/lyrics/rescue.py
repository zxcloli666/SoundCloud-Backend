from __future__ import annotations

from collections.abc import Sequence

from worker.domain.deadline import Deadline
from worker.domain.lyrics.align import ctc_align
from worker.domain.lyrics.placement import LineTiming, Word, timings_from_words
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.ports import Engines, Float32Array
from worker.observability.counters import Counters

WRONG_TEXT_SCORE_CEILING = 0.45


async def global_ctc(
    engines: Engines,
    vocals: Float32Array,
    lines: Sequence[TokenizedLine],
    counters: Counters,
    deadline: Deadline,
) -> tuple[dict[int, LineTiming], float]:
    deadline.check("rescue")
    tokens, alignment = await ctc_align(engines, vocals, lines, counters, "rescue", deadline)
    if not tokens:
        return {}, 0.0
    counters.inc("align_engine_total", engine="global")
    words = [
        Word(line, text, round(span.start_s, 3), round(span.end_s, 3), span.score)
        for (line, text), span in zip(tokens, alignment.spans, strict=True)
    ]
    placement = timings_from_words(words, "global", None, alignment.score, 0.0)
    return placement, alignment.score


def ctc_agreement(score: float, min_agreement: float) -> float:
    return min(1.0, max(0.0, score) * min_agreement / WRONG_TEXT_SCORE_CEILING)
