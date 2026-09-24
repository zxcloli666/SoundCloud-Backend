from __future__ import annotations

import hashlib

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from worker.domain.deadline import Deadline
from worker.domain.encode_text import EncodeTextLane
from worker.domain.outcome import LeaseDropped, Outcome, Reason, Status, TransientFailure
from worker.observability.counters import Counters


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


async def encode(engines: FakeEngines, model: str, text: str, digest: str | None = None) -> Outcome:
    request = {"model": model, "text": text, "hash": digest or sha256(text)}
    return await EncodeTextLane(engines, Counters()).process(request, Deadline.after(10))


async def test_mulan_query_is_512(engines: FakeEngines) -> None:
    outcome = await encode(engines, "mulan", "грустная гитара под дождём")

    assert outcome.status is Status.OK
    assert len(outcome.fields["vector"]) == 512
    assert np.linalg.norm(outcome.fields["vector"]) == pytest.approx(1.0, abs=1e-5)
    assert engines.calls == [("embed_text_mulan", {"texts": ["грустная гитара под дождём"]})]


async def test_lyrics_query_is_1024_with_query_prompt(engines: FakeEngines) -> None:
    outcome = await encode(engines, "lyrics", "i will survive")

    assert len(outcome.fields["vector"]) == 1024
    assert engines.calls == [("embed_text", {"texts": ["i will survive"], "kind": "query"})]


async def test_hash_mismatch_is_terminal(engines: FakeEngines) -> None:
    outcome = await encode(engines, "lyrics", "i will survive", sha256("i will survive "))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.HASH_MISMATCH)
    assert outcome.fields == {"vector": None}
    assert engines.calls == []


async def test_hash_is_over_utf8_bytes(engines: FakeEngines) -> None:
    outcome = await encode(engines, "mulan", "ё", hashlib.sha256("ё".encode()).hexdigest())

    assert outcome.reason is Reason.HASH_MISMATCH


@pytest.mark.parametrize("text", ["", "   ", "\t\n"])
async def test_blank_text_is_empty(engines: FakeEngines, text: str) -> None:
    outcome = await encode(engines, "lyrics", text)

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.EMPTY_TEXT)
    assert outcome.fields == {"vector": None}


async def test_hash_is_checked_before_emptiness(engines: FakeEngines) -> None:
    outcome = await encode(engines, "lyrics", "  ", sha256(""))

    assert outcome.reason is Reason.HASH_MISMATCH


@pytest.mark.parametrize("text", ["a" * 513, "ё" * 257, "a" * 20000])
async def test_text_over_512_utf8_bytes_is_invalid(engines: FakeEngines, text: str) -> None:
    outcome = await encode(engines, "mulan", text)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert outcome.fields == {"vector": None}
    assert engines.calls == []


@pytest.mark.parametrize("text", ["a" * 512, "ё" * 256])
async def test_text_of_exactly_512_utf8_bytes_is_encoded(engines: FakeEngines, text: str) -> None:
    outcome = await encode(engines, "lyrics", text)

    assert outcome.status is Status.OK


async def test_a_dropped_lease_reaches_the_runner_uncounted(engines: FakeEngines) -> None:
    engines.fail("embed_text_mulan", LeaseDropped("lease of stream_seq 7 is stale"))
    counters = Counters()
    request = {"model": "mulan", "text": "rock", "hash": sha256("rock")}

    with pytest.raises(LeaseDropped):
        await EncodeTextLane(engines, counters).process(request, Deadline.after(10))

    assert counters.total("lane_internal_errors_total") == 0


async def test_unknown_model_is_invalid(engines: FakeEngines) -> None:
    outcome = await encode(engines, "clap", "rock")

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert outcome.fields == {"vector": None}


async def test_engine_failure_keeps_null_vector(engines: FakeEngines) -> None:
    engines.fail("embed_text_mulan", TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=mulan"))

    outcome = await encode(engines, "mulan", "rock")

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)
    assert outcome.fields == {"vector": None}


async def test_wrong_dimension_is_invalid_output(engines: FakeEngines) -> None:
    engines.overrides["embed_text_mulan"] = lambda **_: np.ones((1, 1024), np.float32) / 32

    outcome = await encode(engines, "mulan", "rock")

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.MODEL_OUTPUT_INVALID)
