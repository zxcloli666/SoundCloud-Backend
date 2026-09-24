from __future__ import annotations

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from worker.domain.deadline import Deadline
from worker.domain.lyrics import draft
from worker.domain.lyrics.regions import Region
from worker.domain.ports import Draft
from worker.observability.counters import Counters
from worker.settings import AsrSettings

ASR = AsrSettings(
    rate_cjk=12,
    rate_other=10,
    token_margin=24,
    loop_ngram=4,
    loop_share=0.3,
    repetition_penalty=1.1,
)


def test_token_budget_follows_region_length_and_script() -> None:
    assert draft.max_new_tokens(10.0, "ru", ASR) == 124
    assert draft.max_new_tokens(10.0, "zh", ASR) == 144
    assert draft.max_new_tokens(1.2, "en", ASR) == 36


def test_looping_output_is_detected_on_fast_rap_and_cjk() -> None:
    fast_rap = " ".join(["yeah yeah yeah yeah"] * 12)
    assert draft.looped(fast_rap, "en", ASR)
    honest_rap = "we walked along the river when the city lights went down every word you said"
    assert not draft.looped(honest_rap, "en", ASR)
    assert draft.looped("我的心里我的心里我的心里我的心里我的心里", "zh", ASR)
    assert not draft.looped("我的心里只有你没有他你要相信我的情意并不假", "zh", ASR)
    assert not draft.looped("short", "en", ASR)


async def test_draft_regions_maps_budgets_and_drops_loops() -> None:
    engines = FakeEngines()
    engines.drafts = [
        Draft("зашей мне глаза", "ru", 0.95),
        Draft(" ".join(["на на на на"] * 10), "ru", 0.4),
        Draft("   ", "Russian", 0.9),
    ]
    counters = Counters()
    vocals = np.zeros(16_000 * 30, dtype=np.float32)
    regions = [Region(1.0, 6.0), Region(7.0, 10.0), Region(11.0, 20.0)]
    drafts = await draft.draft_regions(
        engines, vocals, regions, "ru", ASR, counters, Deadline.after(10)
    )
    assert [item.text for item in drafts] == ["зашей мне глаза", None, None]
    assert [item.language for item in drafts] == ["ru", "ru", "russian"]
    assert counters.value("asr_loop_total") == 1
    call = next(kwargs for name, kwargs in engines.calls if name == "draft")
    assert call["max_new_tokens"] == [74, 54, 114]
    assert call["language"] == "ru"
    assert call["repetition_penalty"] == 1.1


@pytest.mark.parametrize("heard", [None, "English", "Chinese"])
async def test_loops_are_judged_in_the_forced_language_not_the_heard_one(
    heard: str | None,
) -> None:
    engines = FakeEngines()
    engines.drafts = [Draft("あいしてる" * 7, heard, 0.3)]
    counters = Counters()
    drafts = await draft.draft_regions(
        engines,
        np.zeros(16_000 * 10, dtype=np.float32),
        [Region(1.0, 6.0)],
        "ja",
        ASR,
        counters,
        Deadline.after(10),
    )
    assert drafts[0].text is None
    assert counters.value("asr_loop_total") == 1


async def test_draft_count_mismatch_is_an_error() -> None:
    engines = FakeEngines()
    engines.overrides["draft"] = lambda **kwargs: [Draft("x", "ru", 1.0)]
    with pytest.raises(ValueError, match="1 texts for 2 regions"):
        await draft.draft_regions(
            engines,
            np.zeros(16_000 * 10, dtype=np.float32),
            [Region(0.0, 2.0), Region(3.0, 5.0)],
            "ru",
            ASR,
            Counters(),
            Deadline.after(10),
        )


def test_language_vote_weights_by_duration() -> None:
    drafts = [
        draft.RegionDraft("a", "ru", 0.9),
        draft.RegionDraft("b", "uk", 0.99),
        draft.RegionDraft("c", "ru", 0.7),
        draft.RegionDraft(None, "en", 1.0),
    ]
    regions = [Region(0, 10), Region(10, 25), Region(25, 45), Region(45, 100)]
    code, prob = draft.language_vote(drafts, regions)
    assert code == "ru"
    assert prob == pytest.approx((10 * 0.9 + 20 * 0.7) / 30)
    assert draft.language_vote([], []) == (None, 0.0)
