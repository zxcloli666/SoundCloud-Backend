from __future__ import annotations

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines, even_alignment
from tests.unit.lyrics.conftest import RUSSIAN_LINES, lines_of
from worker.domain.deadline import Deadline
from worker.domain.lyrics import align
from worker.domain.lyrics.regions import Region
from worker.domain.ports import Alignment, EngineUnavailable, TokenSpan
from worker.observability.counters import Counters
from worker.settings import Settings

VOCALS = np.zeros(16_000 * 40, dtype=np.float32)
REGION = Region(10.0, 16.0)


def aligner(engines: FakeEngines, settings: Settings) -> tuple[align.RegionAligner, Counters]:
    counters = Counters()
    return align.RegionAligner(engines, settings.sync, counters), counters


async def test_russian_region_goes_to_qwen_on_the_region_itself(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, counters = aligner(engines, settings)
    lines = lines_of(RUSSIAN_LINES[:2])
    result = await region_aligner.align(VOCALS, 3, REGION, lines, Deadline.after(10))
    assert result.engine == "qwen"
    call = next(kwargs for name, kwargs in engines.calls if name == "align")
    assert call["language"] == "ru"
    assert call["tokens"][:3] == ["Зашей", "мне", "глаза"]
    assert result.words[0].start_s == 10.0
    assert result.words[-1].end_s <= 16.0
    assert {word.line for word in result.words} == {0, 1}
    assert counters.value("align_engine_total", engine="qwen") == 1
    assert result.inside_share == 1.0


async def test_language_outside_qwen_goes_to_mms_with_romanised_tokens(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, counters = aligner(engines, settings)
    lines = lines_of(["Заший мені очі щоб я не бачив тебе"], "uk")
    result = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    assert result.engine == "mms"
    call = next(kwargs for name, kwargs in engines.calls if name == "ctc_align")
    assert call["tokens"][:2] == ["zashy", "meni"]
    assert not any(name == "align" for name, _ in engines.calls)
    assert counters.value("align_engine_total", engine="mms") == 1


async def test_low_qwen_score_triggers_mms_retry_and_the_cleaner_result_wins(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, counters = aligner(engines, settings)
    lines = lines_of(RUSSIAN_LINES[:1])
    tokens = len(lines[0].tokens)

    def collapsed(**kwargs: object) -> Alignment:
        spans = tuple(TokenSpan(1.0, 1.0, 0.2) for _ in range(tokens))
        return Alignment(spans, 0.2)

    engines.overrides["align"] = collapsed
    result = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    assert result.engine == "mms"
    assert counters.value("region_fallback_total", picked="mms") == 1
    assert result.collapsed_share == 0.0


async def test_retry_keeps_qwen_when_mms_is_not_better(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, counters = aligner(engines, settings)
    lines = lines_of(RUSSIAN_LINES[:1])
    tokens = len(lines[0].tokens)
    engines.overrides["align"] = lambda **kwargs: even_alignment(REGION.duration_s, tokens, 0.3)
    result = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    assert result.engine == "qwen"
    assert counters.value("region_fallback_total", picked="qwen") == 1


async def test_words_outside_the_window_count_against_the_region(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, _ = aligner(engines, settings)
    lines = lines_of(["раз два три"])

    def outside(**kwargs: object) -> Alignment:
        return Alignment(
            (TokenSpan(-5.0, -4.0, 0.9), TokenSpan(1.0, 1.5, 0.9), TokenSpan(40.0, 41.0, 0.9)), 0.9
        )

    engines.overrides["align"] = outside
    engines.overrides["ctc_align"] = outside
    result = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    assert result.inside_share == pytest.approx(1 / 3)
    assert result.words[0].start_s == 10.0
    assert result.words[-1].end_s == 16.0


async def test_broken_align_slot_sends_every_region_to_mms(
    engines: FakeEngines, settings: Settings
) -> None:
    region_aligner, counters = aligner(engines, settings)
    engines.fail("align", EngineUnavailable("align", "broken"))
    lines = lines_of(RUSSIAN_LINES[:1])
    first = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    second = await region_aligner.align(VOCALS, 1, Region(20.0, 25.0), lines, Deadline.after(10))
    assert (first.engine, second.engine) == ("mms", "mms")
    assert sum(1 for name, _ in engines.calls if name == "align") == 1
    assert counters.value("align_engine_unavailable_total", slot="align") == 1


async def test_region_fallback_can_be_disabled(engines: FakeEngines, settings: Settings) -> None:
    from dataclasses import replace

    sync = replace(settings.sync, align=replace(settings.sync.align, region_fallback=False))
    region_aligner = align.RegionAligner(engines, sync, Counters())
    lines = lines_of(RUSSIAN_LINES[:1])
    engines.overrides["align"] = lambda **kwargs: even_alignment(10.0, len(lines[0].tokens), 0.1)
    result = await region_aligner.align(VOCALS, 0, REGION, lines, Deadline.after(10))
    assert result.engine == "qwen"
    assert not any(name == "ctc_align" for name, _ in engines.calls)


async def test_span_count_mismatch_is_an_error(engines: FakeEngines, settings: Settings) -> None:
    region_aligner, _ = aligner(engines, settings)
    engines.overrides["align"] = lambda **kwargs: Alignment((TokenSpan(0, 1, 1),), 1.0)
    with pytest.raises(ValueError, match="1 spans for"):
        await region_aligner.align(VOCALS, 0, REGION, lines_of(["раз два"]), Deadline.after(10))


@pytest.mark.parametrize(
    ("seconds", "aligned"),
    [(3.0, True), (2.0, False)],
)
async def test_ctc_text_needs_two_frames_per_letter_and_per_star(
    engines: FakeEngines, seconds: float, aligned: bool
) -> None:
    counters = Counters()
    lines = lines_of(["ab cd ef gh ij kl mn op qr st uv wx yz ab cd ef gh ij kl mn op qr st"])
    clip = VOCALS[: int(16_000 * seconds)]
    tokens, alignment = await align.ctc_align(
        engines, clip, lines, counters, "region", Deadline.after(10)
    )
    assert bool(tokens) is aligned
    assert (len(alignment.spans) > 0) is aligned
    assert counters.value("ctc_text_too_long_total", stage="region") == (0 if aligned else 1)


def test_majority_language_weighs_by_units() -> None:
    lines = lines_of(["раз два три четыре"], "ru") + lines_of(["one"], "en")
    assert align.majority_language(lines) == "ru"
    assert align.majority_language([]) is None
