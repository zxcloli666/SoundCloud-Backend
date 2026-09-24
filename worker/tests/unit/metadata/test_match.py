from __future__ import annotations

from collections.abc import Mapping

import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.llm import FakeRefiner
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.metadata.match import (
    MATCH_SCHEMA,
    Track,
    TrackMatcher,
    duration_closeness,
    score,
    token_set,
)
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.observability.counters import Counters

REPLY = "ai.rpc.match_track.reply"
TARGET = {"artist": "Billie Eilish", "title": "Ocean Eyes"}


def matcher(refiner: FakeRefiner | None = None) -> tuple[TrackMatcher, Counters]:
    counters = Counters()
    return TrackMatcher(refiner, counters), counters


def deadline(clock: FakeClock, seconds: float = 10.0) -> Deadline:
    return Deadline(clock.now() + seconds, clock.now)


def request(*candidates: tuple[int, str, str, float | None]) -> dict[str, object]:
    return {
        "target": TARGET,
        "candidates": [
            {"id": ident, "artist": artist, "title": title, "duration_sec": duration}
            for ident, artist, title, duration in candidates
        ],
    }


async def test_a_clear_winner_is_answered_without_the_llm(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    subject, _ = matcher(refiner)

    data = await subject.match(
        request((1, "Someone", "Different Song", 200), (2, "Billie Eilish", "Ocean Eyes", 200)),
        deadline(clock),
    )

    assert data["match_id"] == 2
    assert data["confidence"] >= 0.85
    assert data["source"] == "deterministic"
    assert refiner.calls == []


async def test_veto_removes_other_versions(clock: FakeClock) -> None:
    subject, _ = matcher()

    data = await subject.match(
        request(
            (1, "Billie Eilish", "Ocean Eyes (Skrillex Remix)", None),
            (2, "Billie Eilish", "Ocean Eyes (Live)", None),
            (3, "Billie Eilish", "Ocean Eyes - Sped Up", None),
        ),
        deadline(clock),
    )

    assert data == {"match_id": None, "confidence": 0.0, "source": "deterministic"}


async def test_a_wanted_version_matches_the_same_version(clock: FakeClock) -> None:
    subject, _ = matcher()

    data = await subject.match(
        {
            "target": {"artist": "Billie Eilish", "title": "Ocean Eyes (Live)"},
            "candidates": [
                {"id": 1, "artist": "Billie Eilish", "title": "Ocean Eyes"},
                {"id": 2, "artist": "Billie Eilish", "title": "Ocean Eyes (Live)"},
            ],
        },
        deadline(clock),
    )

    assert data["match_id"] == 2


async def test_a_weak_best_is_no_match(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    subject, _ = matcher(refiner)

    data = await subject.match(request((1, "Nobody", "Something Else", 90)), deadline(clock))

    assert data["match_id"] is None
    assert data["confidence"] < 0.5
    assert refiner.calls == []


async def test_a_tie_is_null_with_the_best_confidence(clock: FakeClock) -> None:
    subject, _ = matcher()

    data = await subject.match(
        request((1, "Billie Eilish", "Ocean Eyes", 200), (2, "Billie Eilish", "Ocean Eyes", 200)),
        deadline(clock),
    )

    assert data["match_id"] is None
    assert data["confidence"] > 0.9
    assert data["source"] == "deterministic"


async def test_a_borderline_pick_is_refined_by_the_llm(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with({"match_id": 2, "confidence": 0.8})
    subject, counters = matcher(refiner)

    data = await subject.match(
        request((1, "Billie Eilish", "Ocean Eyes", 200), (2, "billie", "Ocean Eyes", 201)),
        deadline(clock),
    )

    assert data == {"match_id": 2, "confidence": 0.8, "source": "llm"}
    assert refiner.calls[0][1] is MATCH_SCHEMA
    assert "id=1" in refiner.calls[0][0] and "id=2" in refiner.calls[0][0]
    assert counters.value("rpc_answers_total", method="match_track", source="llm") == 1


async def test_the_llm_may_answer_no_match(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with({"match_id": None, "confidence": 0.7})
    subject, _ = matcher(refiner)

    data = await subject.match(
        request((1, "Billie Eilish", "Ocean Eyes", 200), (2, "billie", "Ocean Eyes", 201)),
        deadline(clock),
    )

    assert data == {"match_id": None, "confidence": 0.7, "source": "llm"}


@pytest.mark.parametrize("picked", [7, 3])
async def test_ids_outside_the_shortlist_or_vetoed_are_ungrounded(
    clock: FakeClock, picked: int
) -> None:
    refiner = FakeRefiner()
    refiner.reply_with({"match_id": picked, "confidence": 0.99})
    subject, counters = matcher(refiner)

    data = await subject.match(
        request(
            (1, "Billie Eilish", "Ocean Eyes", 200),
            (2, "billie", "Ocean Eyes", 201),
            (3, "Billie Eilish", "Ocean Eyes (Remix)", 200),
        ),
        deadline(clock),
    )

    assert refiner.ungrounded == 1
    assert data["source"] == "deterministic"
    assert counters.value("rpc_llm_fallbacks_total", method="match_track") == 1


async def test_bool_ids_from_the_llm_are_rejected(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with({"match_id": True, "confidence": 0.9})
    subject, _ = matcher(refiner)

    data = await subject.match(
        request((1, "Billie Eilish", "Ocean Eyes", 200), (2, "billie", "Ocean Eyes", 201)),
        deadline(clock),
    )

    assert data["source"] == "deterministic"


async def test_short_deadline_keeps_the_deterministic_answer(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    subject, counters = matcher(refiner)

    await subject.match(
        request((1, "Billie Eilish", "Ocean Eyes", 200), (2, "billie", "Ocean Eyes", 201)),
        deadline(clock, 3.0),
    )

    assert refiner.calls == []
    assert counters.value("llm_skipped_total", method="match_track", why="short_deadline") == 1


@pytest.mark.parametrize(
    "request_",
    [
        {"target": TARGET, "candidates": []},
        {"target": TARGET},
        {"target": "x", "candidates": [{"id": 1, "artist": "a", "title": "b"}]},
        {"target": TARGET, "candidates": [{"id": True, "artist": "a", "title": "b"}]},
        {"target": TARGET, "candidates": ["x"]},
        request((-1, "Billie Eilish", "Ocean Eyes", 200)),
        request((2**32, "Billie Eilish", "Ocean Eyes", 200)),
        request((2**40, "Billie Eilish", "Ocean Eyes", 200)),
        request(*[(ident, "Billie Eilish", "Ocean Eyes", 200) for ident in range(51)]),
    ],
)
async def test_malformed_requests_are_invalid(
    clock: FakeClock, request_: Mapping[str, object]
) -> None:
    subject, _ = matcher()

    with pytest.raises(PermanentFailure) as failure:
        await subject.match(request_, deadline(clock))

    assert failure.value.reason is Reason.INVALID_REQUEST


async def test_the_u32_edge_id_and_fifty_candidates_are_accepted(
    clock: FakeClock, contract: Contract
) -> None:
    subject, _ = matcher()
    others = [(ident, "Someone", f"Song {ident}", 100) for ident in range(49)]

    data = await subject.match(
        request(*others, (2**32 - 1, "Billie Eilish", "Ocean Eyes", 200)), deadline(clock)
    )

    assert data["match_id"] == 2**32 - 1
    assert contract.validate(REPLY, {"ok": True, "data": dict(data)}) == []


async def test_answers_match_the_reply_schema(clock: FakeClock, contract: Contract) -> None:
    subject, _ = matcher()

    data = await subject.match(request((1, "Billie Eilish", "Ocean Eyes", 200)), deadline(clock))

    assert contract.validate(REPLY, {"ok": True, "data": dict(data)}) == []


async def test_an_expired_call_is_a_deadline_failure(clock: FakeClock) -> None:
    subject, _ = matcher()

    with pytest.raises(TransientFailure) as failure:
        await subject.match(request((1, "a", "b", None)), deadline(clock, 0.4))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED


def test_score_weights_artist_title_and_duration() -> None:
    target = Track("Billie Eilish", "Ocean Eyes", 200)

    assert score(target, Track("BILLIE EILISH", "ocean eyes!", 200)) == pytest.approx(1.0)
    assert score(target, Track("Billie Eilish", "Ocean Eyes", None)) == pytest.approx(0.95)
    assert score(target, Track("Billie Eilish", "Ocean Eyes", 400)) == pytest.approx(0.9)


def test_uploader_style_candidates_use_the_artist_from_the_title() -> None:
    target = Track("Billie Eilish", "Ocean Eyes", None)

    assert score(target, Track("Nightcore Vibes", "Billie Eilish - Ocean Eyes", None)) > 0.9


def test_token_set_ignores_word_order() -> None:
    assert token_set("Ocean Eyes Billie", "Billie Ocean Eyes") == pytest.approx(1.0)
    assert token_set("", "anything") == 0.0


@pytest.mark.parametrize(
    ("target", "candidate", "closeness"),
    [(200, 203, 1.0), (200, 225, 0.0), (200, 214.5, 0.5), (None, 200, 0.5)],
)
def test_duration_closeness(target: float | None, candidate: float, closeness: float) -> None:
    assert duration_closeness(target, candidate) == pytest.approx(closeness)
