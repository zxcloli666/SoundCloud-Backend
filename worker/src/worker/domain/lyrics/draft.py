from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass

from worker.domain.deadline import Deadline
from worker.domain.language import to_internal
from worker.domain.lyrics import regions as region_tools
from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import is_cjk, tokenize
from worker.domain.ports import Engines, Float32Array
from worker.observability.counters import Counters
from worker.settings import AsrSettings


@dataclass(frozen=True)
class RegionDraft:
    text: str | None
    language: str | None
    language_prob: float


async def draft_regions(
    engines: Engines,
    vocals: Float32Array,
    regions: Sequence[Region],
    language: str,
    settings: AsrSettings,
    counters: Counters,
    deadline: Deadline,
) -> list[RegionDraft]:
    deadline.check("draft")
    clips = [region_tools.clip(vocals, region.start_s, region.end_s)[0] for region in regions]
    budgets = [max_new_tokens(region.duration_s, language, settings) for region in regions]
    drafts = await engines.draft(clips, language, budgets, settings.repetition_penalty, deadline)
    if len(drafts) != len(regions):
        raise ValueError(f"draft returned {len(drafts)} texts for {len(regions)} regions")
    result: list[RegionDraft] = []
    for draft in drafts:
        text: str | None = draft.text.strip()
        if text and looped(text, language, settings):
            counters.inc("asr_loop_total")
            text = None
        result.append(RegionDraft(text or None, to_internal(draft.language), draft.language_prob))
    return result


def max_new_tokens(duration_s: float, language: str | None, settings: AsrSettings) -> int:
    rate = settings.rate_cjk if is_cjk(language) else settings.rate_other
    return math.ceil(duration_s * rate) + settings.token_margin


def looped(text: str, language: str | None, settings: AsrSettings) -> bool:
    units = tokenize(text, to_internal(language))
    if is_cjk(to_internal(language)):
        units = [character for token in units for character in token]
    size = settings.loop_ngram
    if len(units) < 2 * size:
        return False
    grams = [tuple(units[i : i + size]) for i in range(len(units) - size + 1)]
    repeated = 1.0 - len(set(grams)) / len(grams)
    return repeated > settings.loop_share


def language_vote(
    drafts: Sequence[RegionDraft], regions: Sequence[Region]
) -> tuple[str | None, float]:
    weight: dict[str, float] = {}
    confidence: dict[str, float] = {}
    for draft, region in zip(drafts, regions, strict=True):
        if draft.language is None or draft.text is None:
            continue
        weight[draft.language] = weight.get(draft.language, 0.0) + region.duration_s
        confidence[draft.language] = (
            confidence.get(draft.language, 0.0) + region.duration_s * draft.language_prob
        )
    if not weight:
        return None, 0.0
    winner = max(weight, key=lambda code: weight[code])
    return winner, confidence[winner] / weight[winner]
