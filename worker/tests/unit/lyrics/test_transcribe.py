from __future__ import annotations

import numpy as np
import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.engines import even_alignment
from tests.unit.lyrics.conftest import (
    RUSSIAN_LINES,
    LaneHarness,
    inside_region_alignment,
    lines_of,
)
from worker.domain.deadline import Deadline
from worker.domain.language import LanguageDetection
from worker.domain.lyrics import anchors, rescue, transcribe
from worker.domain.lyrics.draft import RegionDraft
from worker.domain.lyrics.regions import Region
from worker.domain.outcome import LeaseDropped, Reason, Status
from worker.domain.ports import Alignment, Draft, EngineUnavailable, LanguageGuess, TokenSpan

TEXT = "[Verse 1]\n" + "\n".join(RUSSIAN_LINES[:2]) + "\n(Chorus)\n" + "\n".join(RUSSIAN_LINES[2:])
UKRAINIAN = "\n".join(
    [
        "Заший мені очі щоб я не бачив тебе",
        "Тіло гниє заростаючи в квітах",
        "Просто забудь мене",
        "Ми сяємо як востаннє тут",
        "Тобі з нами не можна",
    ]
)
RESULT_FIELDS = {
    "sync_version",
    "confidence",
    "placed_share",
    "aligned_share",
    "lines_total",
    "lines_unplaced",
    "language",
}


def lrc_text_lines(outcome_fields: dict[str, object]) -> list[str]:
    lrc = str(outcome_fields["synced_lrc"])
    return [line[10:] for line in lrc.split("\n") if len(line) > 10]


async def test_a_russian_track_is_synced_and_published(harness: LaneHarness) -> None:
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert outcome.status is Status.OK, outcome.detail
    fields = dict(outcome.fields)
    assert RESULT_FIELDS | {"synced_lrc", "words"} <= set(fields)
    assert fields["sync_version"] == "s4.c07281df.49402e95.deadbeef"
    assert fields["language"] == "ru"
    assert fields["lines_total"] == 5 and fields["lines_unplaced"] == 0
    assert fields["placed_share"] == 1.0 and fields["aligned_share"] == 1.0
    assert lrc_text_lines(fields) == RUSSIAN_LINES
    assert str(fields["synced_lrc"]).count("\n") == 6
    assert all(not word["unplaced"] for word in fields["words"])
    called = [name for name, _ in harness.engines.calls]
    assert called[:4] == ["detect_language", "separate", "vad", "draft"]
    assert called.count("align") == 5
    assert harness.counters.value("align_engine_total", engine="qwen") == 5
    assert harness.counters.value("transcribe_strategy_total", strategy="anchored") == 1


async def test_text_without_sung_lines_is_empty(harness: LaneHarness) -> None:
    outcome = await harness.lane.process(
        harness.request("[Intro]\n\n(Instrumental)"), Deadline.after(60)
    )
    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.EMPTY_REFERENCE_TEXT)
    assert outcome.fields == {"sync_version": "s4.c07281df.49402e95.deadbeef"}
    assert not harness.server.requests


async def test_no_speech_is_rejected_with_zero_metrics(harness: LaneHarness) -> None:
    harness.engines.regions = []
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.NO_VOCAL_DETECTED)
    assert set(outcome.fields) == RESULT_FIELDS
    assert outcome.fields["placed_share"] == 0.0
    assert outcome.fields["lines_unplaced"] == 5
    assert outcome.fields["language"] == "ru"


@pytest.mark.parametrize(
    ("text", "language", "unsupported"),
    [
        ("\n".join(RUSSIAN_LINES), "ru", False),
        ("今日はいい天気です\n明日も晴れるでしょう", "ja", False),
        ("გამარჯობა მეგობარო\nროგორ ხარ დღეს", "ka", False),
        ("ሰላም ወንድሜ\nእንዴት ነህ ዛሬ", "am", False),
        ("𒀭𒂗𒆤 𒀭𒈾𒀀\n𒄑𒉈𒂵 𒀭𒌓", None, True),
    ],
)
async def test_unsupported_language_needs_a_failed_romanisation(
    harness: LaneHarness, text: str, language: str | None, unsupported: bool
) -> None:
    outcome = await harness.lane.process(
        harness.request(text, language=language), Deadline.after(60)
    )
    assert (outcome.reason is Reason.UNSUPPORTED_LANGUAGE) == unsupported
    if unsupported:
        assert outcome.status is Status.REJECTED
        assert set(outcome.fields) == RESULT_FIELDS
        assert not harness.server.requests


async def test_language_outside_asr_skips_the_draft_and_never_mismatches(
    harness: LaneHarness,
) -> None:
    harness.engines.default_language = []
    harness.engines.languages = {line: [] for line in UKRAINIAN.split("\n")}
    harness.engines.drafts = [Draft("зашей мне глаза", "ru", 0.99)] * 5
    outcome = await harness.lane.process(
        harness.request(UKRAINIAN, language="uk"), Deadline.after(60)
    )
    assert outcome.reason is not Reason.LYRICS_MISMATCH
    assert outcome.status is Status.OK, outcome.detail
    called = [name for name, _ in harness.engines.calls]
    assert "draft" not in called and "align" not in called
    assert called.count("ctc_align") == 1
    assert harness.counters.value("align_engine_total", engine="global") == 1
    assert harness.counters.value("transcribe_strategy_total", strategy="global") == 1
    assert outcome.fields["language"] == "uk"


async def test_global_ctc_score_under_the_wrong_text_ceiling_is_a_mismatch(
    harness: LaneHarness,
) -> None:
    harness.engines.default_language = []
    harness.engines.languages = {line: [] for line in UKRAINIAN.split("\n")}

    def weak(**kwargs: object) -> Alignment:
        return even_alignment(40.0, len(list(kwargs["tokens"])), 0.44)

    harness.engines.overrides["ctc_align"] = weak
    outcome = await harness.lane.process(
        harness.request(UKRAINIAN, language="uk"), Deadline.after(60)
    )
    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.LYRICS_MISMATCH)
    assert outcome.detail == "anchor_agreement=0.29 language_agreement=None"


@pytest.mark.parametrize(
    ("score", "agreement"), [(0.0, 0.0), (0.45, 0.3), (0.9, 0.6), (1.5, 1.0), (-0.2, 0.0)]
)
def test_ctc_agreement_maps_the_wrong_text_ceiling_onto_the_gate(
    score: float, agreement: float
) -> None:
    assert rescue.ctc_agreement(score, 0.30) == pytest.approx(agreement)


async def test_foreign_lyrics_are_rejected_as_mismatch_without_rescue(
    harness: LaneHarness,
) -> None:
    harness.engines.drafts = [
        Draft("we walked along the river when the city lights went down", "en", 0.99),
        Draft("every word you never said still echoes in this town", "en", 0.99),
        Draft("hold me closer than the night and never let me go", "en", 0.99),
        Draft("the morning comes too early", "en", 0.99),
        Draft("and the evening comes too slow", "en", 0.99),
    ]
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.LYRICS_MISMATCH)
    assert outcome.detail is not None and "anchor_agreement=" in outcome.detail
    assert harness.counters.value("rescue_total", accepted="true") == 0
    assert harness.counters.value("rescue_total", accepted="false") == 0


async def test_lines_cut_by_jobs_beyond_five_percent_reject_the_track(
    harness: LaneHarness,
) -> None:
    outcome = await harness.lane.process(
        harness.request(TEXT, reference_lines_total=8), Deadline.after(60)
    )
    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.TOO_FEW_LINES_PLACED)
    assert outcome.fields["lines_total"] == 6 and outcome.fields["lines_unplaced"] == 1
    assert outcome.fields["placed_share"] == pytest.approx(5 / 6, abs=1e-3)


async def test_collapsed_region_alignment_is_rescued_by_global_ctc(
    harness: LaneHarness,
) -> None:
    total_tokens = sum(len(line.split()) for line in RUSSIAN_LINES)

    def collapsed(**kwargs: object) -> Alignment:
        count = len(list(kwargs["tokens"]))
        return Alignment(tuple(TokenSpan(1.0, 1.0, 0.1) for _ in range(count)), 0.1)

    def global_only(**kwargs: object) -> Alignment:
        count = len(list(kwargs["tokens"]))
        if count >= total_tokens:
            return even_alignment(40.0, count, 0.8)
        return collapsed(**kwargs)

    harness.engines.overrides["align"] = collapsed
    harness.engines.overrides["ctc_align"] = global_only
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert outcome.status is Status.OK, outcome.detail
    assert harness.counters.value("align_engine_total", engine="global") == 1
    assert harness.counters.value("rescue_total", accepted="true") == 1
    assert harness.counters.value("region_fallback_total", picked="qwen") == 5


async def test_separator_failure_falls_back_to_the_mix(harness: LaneHarness) -> None:
    harness.engines.fail("separate", EngineUnavailable("sep", "restarting"))
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert outcome.status is Status.OK, outcome.detail
    assert harness.counters.value("separation_fallback_total") == 1


async def test_words_are_withheld_when_too_many_collapse(harness: LaneHarness) -> None:
    def partly_collapsed(**kwargs: object) -> Alignment:
        clean = inside_region_alignment(**kwargs)
        spans = [
            TokenSpan(span.start_s, span.start_s if index % 7 == 0 else span.end_s, span.score)
            for index, span in enumerate(clean.spans)
        ]
        return Alignment(tuple(spans), clean.score)

    harness.engines.overrides["align"] = partly_collapsed
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert outcome.status is Status.OK, outcome.detail
    assert "words" not in outcome.fields
    assert "synced_lrc" in outcome.fields


async def test_missing_audio_is_reported_as_missing(harness: LaneHarness) -> None:
    outcome = await harness.lane.process(
        harness.request(TEXT, audio_url=harness.server.url("/missing")), Deadline.after(60)
    )
    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.AUDIO_NOT_FOUND)
    assert outcome.fields == {"sync_version": "s4.c07281df.49402e95.deadbeef"}


async def test_expired_deadline_fails_transiently(harness: LaneHarness) -> None:
    clock = FakeClock()
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(0, clock.now))
    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)
    assert outcome.fields["sync_version"] == "s4.c07281df.49402e95.deadbeef"


@pytest.mark.parametrize(
    "broken",
    [
        {"reference_lines_total": 0},
        {"reference_lines_total": "7"},
        {"reference_lines_total": True},
        {"reference_text": None},
        {"language": 5},
        {"audio_url": None},
    ],
)
async def test_invalid_request_is_a_deterministic_failure(
    harness: LaneHarness, broken: dict[str, object]
) -> None:
    outcome = await harness.lane.process(harness.request(TEXT, **broken), Deadline.after(60))
    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert not harness.server.requests


async def test_unexpected_errors_become_internal_error(harness: LaneHarness) -> None:
    harness.engines.fail("vad", RuntimeError("boom"))
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INTERNAL_ERROR)
    assert (
        harness.counters.value(
            "lane_internal_errors_total", lane="transcribe", error="RuntimeError"
        )
        == 1
    )


async def test_a_dropped_lease_reaches_the_runner_uncounted(harness: LaneHarness) -> None:
    harness.engines.fail("vad", LeaseDropped("lease of stream_seq 7 is stale"))
    with pytest.raises(LeaseDropped):
        await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert harness.counters.total("lane_internal_errors_total") == 0


async def test_unavailable_asr_slot_is_a_transient_failure(harness: LaneHarness) -> None:
    harness.engines.fail("draft", EngineUnavailable("asr", "broken"))
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.ENGINE_CRASHED)


@pytest.mark.parametrize(
    ("text_language", "text_prob", "draft", "agreement"),
    [
        ("ru", 0.95, RegionDraft("a", "ru", 0.9), 1.0),
        ("ru", 0.95, RegionDraft("a", "en", 0.9), 0.0),
        ("ru", 0.5, RegionDraft("a", "en", 0.9), None),
        ("ru", 0.95, RegionDraft("a", "en", 0.5), None),
        ("ru", 0.95, RegionDraft(None, "en", 0.9), None),
        ("uk", 0.95, RegionDraft("a", "ru", 0.9), None),
        ("zh", 0.95, RegionDraft("a", "yue", 0.9), 1.0),
        ("yue", 0.95, RegionDraft("a", "zh", 0.9), 1.0),
        ("id", 0.95, RegionDraft("a", "ms", 0.9), 1.0),
        ("ms", 0.95, RegionDraft("a", "id", 0.9), 1.0),
    ],
)
def test_language_agreement_rules(
    text_language: str, text_prob: float, draft: RegionDraft, agreement: float | None
) -> None:
    detection = LanguageDetection(text_language, [text_language], text_prob, 100)
    assert transcribe.language_agreement(detection, [draft], [Region(0.0, 10.0)]) == agreement


async def test_the_requested_language_drives_tokens_and_the_aligner(
    harness: LaneHarness,
) -> None:
    japanese = [
        "東京の夜空に星が輝いて",
        "君の名前を呼ぶ声が響く",
        "遠い約束を忘れないで",
        "明日の朝も君を待つ",
        "心の奥で歌い続ける",
    ]
    harness.engines.default_language = [LanguageGuess("zh", 0.85), LanguageGuess("ja", 0.1)]
    harness.engines.drafts = [Draft(line, "ja", 0.95) for line in japanese]
    outcome = await harness.lane.process(
        harness.request("\n".join(japanese), language="ja"), Deadline.after(60)
    )
    assert outcome.status is Status.OK, outcome.detail
    assert outcome.fields["language"] == "ja"
    languages = {
        kwargs["language"] for name, kwargs in harness.engines.calls if "language" in kwargs
    }
    assert languages == {"ja"}
    first_tokens = next(
        kwargs["tokens"] for name, kwargs in harness.engines.calls if name == "align"
    )
    assert first_tokens == ["東京", "の", "夜空", "に", "星", "が", "輝い", "て"]


async def test_no_draft_at_all_falls_back_to_global_ctc(harness: LaneHarness) -> None:
    harness.engines.drafts = [Draft("", "ru", 0.0)] * 5
    outcome = await harness.lane.process(harness.request(TEXT), Deadline.after(60))
    assert outcome.status is Status.OK, outcome.detail
    assert harness.counters.value("draft_fallback_total") == 1
    assert harness.counters.value("align_engine_total", engine="global") == 1
    assert harness.counters.total("rescue_total") == 0
    assert "align" not in [name for name, _ in harness.engines.calls]


async def test_each_line_keeps_its_own_anchor_similarity(harness: LaneHarness) -> None:
    lines = lines_of(RUSSIAN_LINES[:3])
    regions = [Region(2.0, 8.0)]
    anchoring = anchors.Anchoring(
        (anchors.Assignment(0, 0, (0, 1, 2)),), 0.4, {0: 1.0, 1: 0.0, 2: 0.25}
    )
    vocals = np.zeros(16_000 * 10, dtype=np.float32)
    placement = await harness.lane._align_regions(
        vocals, regions, lines, anchoring, Deadline.after(60)
    )
    assert {line: timing.anchor_similarity for line, timing in placement.items()} == {
        0: 1.0,
        1: 0.0,
        2: 0.25,
    }
