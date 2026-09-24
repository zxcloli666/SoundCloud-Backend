from __future__ import annotations

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from worker.domain.deadline import Deadline
from worker.domain.embed_lyrics import EmbedLyricsLane
from worker.domain.outcome import Outcome, PermanentFailure, Reason, Status, TransientFailure
from worker.domain.ports import EngineUnavailable, LanguageGuess
from worker.observability.counters import Counters

LYRICS = "Я иду по улице ночной и снова думаю о тебе\nГород спит, а я один стою под фонарём"


async def embed(
    engines: FakeEngines, text: str, language: str | None = None, counters: Counters | None = None
) -> Outcome:
    lane = EmbedLyricsLane(engines, counters or Counters())
    request = {"sc_track_id": "98765", "request_id": "lyr:98765:4", "text": text}
    request["language"] = language
    return await lane.process(request, Deadline.after(30))


async def test_embeds_exact_text_as_document(engines: FakeEngines) -> None:
    text = "  " + LYRICS + "\n\n"

    outcome = await embed(engines, text, "ru")

    assert outcome.status is Status.OK
    assert len(outcome.fields["vec"]) == 1024
    assert np.linalg.norm(outcome.fields["vec"]) == pytest.approx(1.0, abs=1e-5)
    assert outcome.fields["language"] == "ru"
    assert engines.calls == [("embed_text", {"texts": [text], "kind": "document"})]


async def test_detects_language_without_hint(engines: FakeEngines) -> None:
    engines.default_language = [LanguageGuess("ru", 0.97)]

    outcome = await embed(engines, LYRICS)

    assert outcome.fields["language"] == "ru"
    assert [name for name, _ in engines.calls] == ["detect_language", "embed_text"]


async def test_detected_language_goes_out_as_iso_639_1(engines: FakeEngines) -> None:
    engines.default_language = [LanguageGuess("tl", 0.97)]

    outcome = await embed(engines, "Mahal kita, ikaw lang ang aking mahal magpakailanman")

    assert outcome.fields["language"] == "tl"


async def test_undetectable_language_is_null(engines: FakeEngines) -> None:
    engines.default_language = [LanguageGuess("ceb", 0.95)]

    outcome = await embed(engines, "Gihigugma tika sa tanan nakong kasingkasing")

    assert outcome.status is Status.OK
    assert outcome.fields["language"] is None


async def test_hint_is_normalised_to_wire(engines: FakeEngines) -> None:
    outcome = await embed(engines, LYRICS, "FIL")

    assert outcome.fields["language"] == "tl"


@pytest.mark.parametrize("text", ["", "   ", "\n\t\n"])
async def test_blank_text_is_empty(engines: FakeEngines, text: str) -> None:
    outcome = await embed(engines, text, "en")

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.EMPTY_TEXT)
    assert outcome.fields == {"language": "en"}
    assert engines.calls == []


async def test_blank_text_without_hint_has_null_language(engines: FakeEngines) -> None:
    outcome = await embed(engines, " ")

    assert outcome.fields == {"language": None}


async def test_text_too_long_for_model_is_terminal(engines: FakeEngines) -> None:
    engines.fail(
        "embed_text", PermanentFailure(Reason.TEXT_TOO_LONG_FOR_MODEL, "tokens=9001 limit=8192")
    )

    outcome = await embed(engines, LYRICS, "ru")

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.TEXT_TOO_LONG_FOR_MODEL)


async def test_wrong_dimension_is_invalid_output(engines: FakeEngines) -> None:
    counters = Counters()
    engines.overrides["embed_text"] = lambda **_: np.ones((1, 768), np.float32) / np.sqrt(768)

    outcome = await embed(engines, LYRICS, "ru", counters)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.MODEL_OUTPUT_INVALID)
    assert counters.value("model_output_invalid_total", slot="text") == 1


@pytest.mark.parametrize(
    ("error", "reason"),
    [
        (TransientFailure(Reason.OUT_OF_MEMORY, "slot=text"), Reason.OUT_OF_MEMORY),
        (EngineUnavailable("text", "broken"), Reason.ENGINE_CRASHED),
        (RuntimeError("boom"), Reason.INTERNAL_ERROR),
    ],
)
async def test_engine_failures(engines: FakeEngines, error: Exception, reason: Reason) -> None:
    engines.fail("embed_text", error)

    outcome = await embed(engines, LYRICS, "ru")

    assert (outcome.status, outcome.reason) == (Status.FAILED, reason)


@pytest.mark.parametrize(
    "error",
    [
        EngineUnavailable("cpu-tools", "restarting"),
        TransientFailure(Reason.OUT_OF_MEMORY, "slot=lid"),
    ],
)
async def test_detection_failure_still_embeds(engines: FakeEngines, error: Exception) -> None:
    counters = Counters()
    engines.fail("detect_language", error)

    outcome = await embed(engines, "君の名前を呼んでいる\n夜空に星が光る", counters=counters)

    assert outcome.status is Status.OK
    assert len(outcome.fields["vec"]) == 1024
    assert outcome.fields["language"] == "ja"
    assert counters.value("language_detect_failures_total", lane="lyrics") == 1


async def test_detection_failure_on_cyrillic_leaves_language_null(engines: FakeEngines) -> None:
    engines.fail("detect_language", EngineUnavailable("cpu-tools", "broken"))

    outcome = await embed(engines, LYRICS)

    assert outcome.status is Status.OK
    assert outcome.fields["language"] is None


async def test_detection_past_the_deadline_is_not_hidden(engines: FakeEngines) -> None:
    engines.fail("detect_language", TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=lid"))

    outcome = await embed(engines, LYRICS)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)


async def test_non_string_text_is_invalid(engines: FakeEngines) -> None:
    outcome = await EmbedLyricsLane(engines, Counters()).process(
        {"sc_track_id": "1", "request_id": "r", "text": 5}, Deadline.after(30)
    )

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
