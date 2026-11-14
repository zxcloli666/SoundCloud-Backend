from __future__ import annotations

import base64
import json
import logging
from collections.abc import Mapping

import numpy as np
import pytest

from tests.conftest import CONTRACT_PATH
from tests.fakes.clock import FakeClock
from tests.fakes.encoder import FAKE_OGG, FakeEncoder
from tests.fakes.engines import SPEECH_SAMPLES, FakeEngines
from worker import contract as contract_module
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.domain.ports import EngineUnavailable, Speech
from worker.domain.speak import ENCODE_MARGIN_S, SAMPLE_RATE, Speaker
from worker.observability.counters import Counters

VOICES = frozenset({"loli", "moriarty"})
TEXT = "Включаю Скриптонита, громкость на половину."
WINDOW_S = 88.5


def request(**overrides: object) -> dict[str, object]:
    return {"voice": "loli", "text": TEXT, **overrides}


def speaker(contract: Contract) -> tuple[Speaker, FakeEngines, FakeEncoder, Counters]:
    engines, encoder, counters = FakeEngines(), FakeEncoder(), Counters()
    return Speaker(engines, encoder, contract, VOICES, counters), engines, encoder, counters


def deadline(clock: FakeClock, seconds: float = WINDOW_S) -> Deadline:
    return Deadline(clock.now() + seconds, clock.now)


async def test_a_line_comes_back_as_the_encoded_clip_and_its_length(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, encoder, counters = speaker(contract)

    data = await subject.say(request(), deadline(clock))

    assert data == {
        "seconds": SPEECH_SAMPLES / SAMPLE_RATE,
        "clip": base64.b64encode(FAKE_OGG).decode(),
    }
    assert base64.b64decode(str(data["clip"]), validate=True) == FAKE_OGG
    assert contract.validate("speak.rpc.say.reply", {"ok": True, "data": dict(data)}) == []
    assert engines.calls == [
        ("say", {"voice": "loli", "text": TEXT, "remaining": WINDOW_S - ENCODE_MARGIN_S})
    ]
    [(samples, remaining)] = encoder.calls
    assert samples is engines.speech.samples
    assert remaining == WINDOW_S
    assert counters.value("speak_said_total", voice="loli") == 1
    assert counters.value("speak_voice_loads_total", voice="loli") == 0


async def test_a_line_that_started_its_voice_counts_the_load(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, _, counters = speaker(contract)
    engines.speech = Speech(np.zeros(SAMPLE_RATE, np.float32), loaded=True, load_s=4.2)

    data = await subject.say(request(voice="moriarty"), deadline(clock))

    assert data["seconds"] == 1.0
    assert counters.value("speak_voice_loads_total", voice="moriarty") == 1


REJECTED: Mapping[str, Mapping[str, object]] = {
    "no voice": {"text": TEXT},
    "no text": {"voice": "loli"},
    "an empty text": request(text=""),
    "an empty voice": request(voice=""),
    "a text that is not a string": request(text=["la"]),
    "a voice that is not a string": request(voice=7),
    "a text over the schema limit": request(text="я" * 301),
}


@pytest.mark.parametrize("asked", REJECTED.values(), ids=list(REJECTED))
async def test_a_request_outside_the_schema_is_invalid_and_never_reaches_the_engine(
    contract: Contract, clock: FakeClock, asked: Mapping[str, object]
) -> None:
    subject, engines, encoder, counters = speaker(contract)

    with pytest.raises(PermanentFailure) as failure:
        await subject.say(asked, deadline(clock))

    assert failure.value.reason is Reason.INVALID_REQUEST
    assert engines.calls == [] and encoder.calls == []
    assert counters.value("speak_rejected_total", why="schema") == 1


async def test_a_text_at_the_schema_limit_is_spoken(contract: Contract, clock: FakeClock) -> None:
    subject, engines, _, _ = speaker(contract)
    await subject.say(request(text="я" * contract.say_limits.text_chars), deadline(clock))
    assert len(engines.calls) == 1


async def test_an_unknown_voice_is_invalid_and_never_reaches_the_engine(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, _, counters = speaker(contract)

    with pytest.raises(PermanentFailure) as failure:
        await subject.say(request(voice="alice"), deadline(clock))

    assert failure.value.reason is Reason.INVALID_REQUEST
    assert engines.calls == []
    assert counters.value("speak_rejected_total", why="voice") == 1


async def test_the_text_limit_is_read_from_the_contract(clock: FakeClock) -> None:
    raw = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    raw["limits"]["say"]["text_chars"] = 10
    subject, engines, _, counters = speaker(contract_module.parse(raw))

    await subject.say(request(text="a" * 10), deadline(clock))
    with pytest.raises(PermanentFailure) as failure:
        await subject.say(request(text="a" * 11), deadline(clock))

    assert failure.value.reason is Reason.INVALID_REQUEST
    assert len(engines.calls) == 1
    assert counters.value("speak_rejected_total", why="text_chars") == 1


async def test_an_engine_failure_keeps_its_reason(contract: Contract, clock: FakeClock) -> None:
    subject, engines, encoder, counters = speaker(contract)
    engines.fail("say", TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=tts expired"))

    with pytest.raises(TransientFailure) as failure:
        await subject.say(request(), deadline(clock))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED
    assert encoder.calls == []
    assert counters.value("speak_failed_total", why="deadline_exceeded") == 1


async def test_an_unavailable_engine_is_an_internal_failure(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, _, counters = speaker(contract)
    engines.fail("say", EngineUnavailable("tts", "broken"))

    with pytest.raises(TransientFailure) as failure:
        await subject.say(request(), deadline(clock))

    assert failure.value.reason is Reason.INTERNAL_ERROR
    assert counters.value("speak_failed_total", why="unavailable") == 1


async def test_a_line_without_audio_fails_and_is_never_encoded(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, encoder, counters = speaker(contract)
    engines.speech = Speech(np.zeros(0, np.float32), loaded=False, load_s=0.0)

    with pytest.raises(PermanentFailure) as failure:
        await subject.say(request(), deadline(clock))

    assert failure.value.reason is Reason.MODEL_OUTPUT_INVALID
    assert encoder.calls == []
    assert counters.value("speak_failed_total", why="no_audio") == 1


@pytest.mark.parametrize(
    "error",
    [
        TransientFailure(Reason.INTERNAL_ERROR, "ffmpeg did not start: no such file"),
        TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=encode"),
    ],
    ids=["missing", "timed out"],
)
async def test_an_encoder_failure_is_an_internal_failure(
    contract: Contract, clock: FakeClock, error: Exception
) -> None:
    subject, _, encoder, counters = speaker(contract)
    encoder.failure = error

    with pytest.raises(TransientFailure) as failure:
        await subject.say(request(), deadline(clock))

    assert failure.value.reason is Reason.INTERNAL_ERROR
    assert counters.value("speak_failed_total", why="encode") == 1


async def test_the_text_is_never_logged(
    contract: Contract, clock: FakeClock, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    subject, _, encoder, _ = speaker(contract)

    await subject.say(request(), deadline(clock))
    with pytest.raises(PermanentFailure):
        await subject.say(request(voice="alice"), deadline(clock))
    with pytest.raises(PermanentFailure):
        await subject.say(request(text=TEXT * 20), deadline(clock))
    encoder.failure = TransientFailure(Reason.INTERNAL_ERROR, "ffmpeg rc=1")
    with pytest.raises(TransientFailure):
        await subject.say(request(), deadline(clock))

    assert {record.getMessage() for record in caplog.records} >= {
        "speak_said",
        "speak_rejected",
        "speak_failed",
    }
    logged = " ".join(f"{record.getMessage()} {record.__dict__}" for record in caplog.records)
    assert "Скриптонита" not in logged


async def test_a_clip_over_the_reply_schema_is_an_internal_failure(clock: FakeClock) -> None:
    raw = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    clip = raw["schemas"]["speak.rpc.say.reply"]["$defs"]["SayData"]["properties"]["clip"]
    clip["maxLength"] = len(base64.b64encode(FAKE_OGG)) - 1
    subject, _, _, counters = speaker(contract_module.parse(raw))

    with pytest.raises(TransientFailure) as failure:
        await subject.say(request(), deadline(clock))

    assert failure.value.reason is Reason.INTERNAL_ERROR
    assert counters.value("speak_failed_total", why="reply") == 1
    assert counters.value("speak_said_total", voice="loli") == 0
