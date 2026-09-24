from __future__ import annotations

import logging
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass

from worker.domain.deadline import Deadline
from worker.domain.language import ALIGNER_LANGUAGES
from worker.domain.lyrics import regions as region_tools
from worker.domain.lyrics.placement import Engine, Word
from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.ports import Alignment, Engines, EngineUnavailable, Float32Array
from worker.observability.counters import Counters
from worker.settings import SyncSection

FALLBACK_COLLAPSED_SHARE = 0.5
MMS_FRAMES_PER_S = 50.0
FRAMES_PER_CTC_TARGET = 2
QWEN_WINDOW_PAD_S = 0.0

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class RegionResult:
    region: int
    engine: Engine
    words: tuple[Word, ...]
    score: float
    inside_share: float
    collapsed_share: float


@dataclass(frozen=True)
class Window:
    start_s: float
    end_s: float
    samples: Float32Array
    offset_s: float

    @property
    def length_s(self) -> float:
        return int(self.samples.shape[0]) / region_tools.SAMPLE_RATE


class RegionAligner:
    def __init__(self, engines: Engines, settings: SyncSection, counters: Counters) -> None:
        self._engines = engines
        self._settings = settings
        self._counters = counters
        self._qwen_available = True

    async def align(
        self,
        vocals: Float32Array,
        index: int,
        region: Region,
        lines: Sequence[TokenizedLine],
        deadline: Deadline,
    ) -> RegionResult:
        deadline.check("align")
        window = cut_window(vocals, region, self._settings.window_pad_s)
        language = majority_language(lines)
        if language in ALIGNER_LANGUAGES and self._qwen_available:
            region_only = cut_window(vocals, region, QWEN_WINDOW_PAD_S)
            primary = await self._qwen(region_only, index, region, lines, language, deadline)
            if primary is not None:
                return await self._maybe_retry(
                    vocals, primary, window, index, region, lines, deadline
                )
        return await self._mms(window, index, region, lines, deadline)

    async def _qwen(
        self,
        window: Window,
        index: int,
        region: Region,
        lines: Sequence[TokenizedLine],
        language: str,
        deadline: Deadline,
    ) -> RegionResult | None:
        tokens = [(line.ordinal, token) for line in lines for token in line.tokens]
        if not tokens:
            return RegionResult(index, "qwen", (), 0.0, 0.0, 0.0)
        try:
            alignment = await self._engines.align(
                window.samples, [token for _, token in tokens], language, deadline
            )
        except EngineUnavailable as error:
            self._qwen_available = False
            self._counters.inc("align_engine_unavailable_total", slot=error.slot)
            log.warning("qwen aligner unavailable, regions go to mms", extra={"state": error.state})
            return None
        self._counters.inc("align_engine_total", engine="qwen")
        return region_result("qwen", index, region, window, tokens, alignment, self._settings)

    async def _mms(
        self,
        window: Window,
        index: int,
        region: Region,
        lines: Sequence[TokenizedLine],
        deadline: Deadline,
    ) -> RegionResult:
        tokens, alignment = await ctc_align(
            self._engines, window.samples, lines, self._counters, "region", deadline
        )
        if tokens:
            self._counters.inc("align_engine_total", engine="mms")
        return region_result("mms", index, region, window, tokens, alignment, self._settings)

    async def _maybe_retry(
        self,
        vocals: Float32Array,
        primary: RegionResult,
        window: Window,
        index: int,
        region: Region,
        lines: Sequence[TokenizedLine],
        deadline: Deadline,
    ) -> RegionResult:
        settings = self._settings.align
        if not settings.region_fallback or not needs_retry(primary, settings.region_min_score):
            return primary
        retry = await self._mms(window, index, region, lines, deadline)
        chosen = pick(primary, retry)
        self._counters.inc("region_fallback_total", picked=chosen.engine)
        return chosen


async def ctc_align(
    engines: Engines,
    samples: Float32Array,
    lines: Sequence[TokenizedLine],
    counters: Counters,
    stage: str,
    deadline: Deadline,
) -> tuple[list[tuple[int, str]], Alignment]:
    tokens = [
        (line.ordinal, token)
        for line in lines
        for token, romanized in zip(line.tokens, line.romanized, strict=True)
        if romanized
    ]
    romanized = [romanized for line in lines for romanized in line.romanized if romanized]
    if not tokens:
        return [], Alignment((), 0.0)
    duration_s = int(samples.shape[0]) / region_tools.SAMPLE_RATE
    targets = sum(len(item) + 1 for item in romanized)
    if targets * FRAMES_PER_CTC_TARGET > MMS_FRAMES_PER_S * duration_s:
        counters.inc("ctc_text_too_long_total", stage=stage)
        log.info(
            "ctc alignment skipped, text longer than the clip can hold",
            extra={"stage": stage, "ctc_targets": targets, "duration_s": round(duration_s, 2)},
        )
        return [], Alignment((), 0.0)
    return tokens, await engines.ctc_align(samples, romanized, deadline)


def cut_window(vocals: Float32Array, region: Region, pad_s: float) -> Window:
    samples, offset = region_tools.clip(vocals, region.start_s - pad_s, region.end_s + pad_s)
    return Window(region.start_s, region.end_s, samples, offset)


def majority_language(lines: Sequence[TokenizedLine]) -> str | None:
    votes: Counter[str | None] = Counter()
    for line in lines:
        votes[line.language] += max(1, line.units)
    if not votes:
        return None
    return max(votes, key=lambda code: (votes[code], code or ""))


def region_result(
    engine: Engine,
    index: int,
    region: Region,
    window: Window,
    tokens: Sequence[tuple[int, str]],
    alignment: Alignment,
    settings: SyncSection,
) -> RegionResult:
    if len(alignment.spans) != len(tokens):
        raise ValueError(f"{engine} returned {len(alignment.spans)} spans for {len(tokens)} tokens")
    words: list[Word] = []
    outside = 0
    for (line, text), span in zip(tokens, alignment.spans, strict=True):
        start = window.offset_s + span.start_s
        end = window.offset_s + span.end_s
        if start < window.offset_s - 1e-6 or start > window.offset_s + window.length_s + 1e-6:
            outside += 1
        start = min(max(start, window.offset_s), window.offset_s + window.length_s)
        end = min(max(end, start), window.offset_s + window.length_s)
        words.append(Word(line, text, round(start, 3), round(end, 3), span.score))
    pad = settings.window_pad_s
    inside = sum(1 for word in words if region.contains(word.start_s, pad))
    collapsed = sum(1 for word in words if word.collapsed)
    count = max(1, len(words))
    return RegionResult(
        region=index,
        engine=engine,
        words=tuple(words),
        score=alignment.score,
        inside_share=(inside - outside) / count if words else 0.0,
        collapsed_share=collapsed / count if words else 0.0,
    )


def needs_retry(result: RegionResult, min_score: float) -> bool:
    return (
        result.score < min_score
        or result.collapsed_share > FALLBACK_COLLAPSED_SHARE
        or result.inside_share < 1.0
    )


def pick(primary: RegionResult, retry: RegionResult) -> RegionResult:
    if retry.inside_share > primary.inside_share:
        return retry
    if (
        retry.inside_share == primary.inside_share
        and retry.collapsed_share < primary.collapsed_share
    ):
        return retry
    return primary
