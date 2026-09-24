from __future__ import annotations

from collections.abc import Mapping

import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.llm import FakeRefiner
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.metadata.resolve import RESOLVE_SCHEMA, ArtistResolver
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.llm.validate import validate
from worker.observability.counters import Counters

REPLY = "ai.rpc.resolve_artist.reply"


def resolver(refiner: FakeRefiner | None = None) -> tuple[ArtistResolver, Counters]:
    counters = Counters()
    return ArtistResolver(refiner, counters), counters


def deadline(clock: FakeClock, seconds: float = 20.0) -> Deadline:
    return Deadline(clock.now() + seconds, clock.now)


def llm_reply(**overrides: object) -> dict[str, object]:
    reply: dict[str, object] = {
        "primary_artist": "Kendrick Lamar",
        "featured": ["SZA"],
        "producers": [],
        "remixers": [],
        "album": None,
        "confidence": 0.93,
    }
    reply.update(overrides)
    return reply


async def test_metadata_artist_wins_with_its_featured(clock: FakeClock) -> None:
    subject, _ = resolver()

    data = await subject.resolve(
        {"title": "luther", "uploader": "somebody", "metadata_artist": "Kendrick Lamar ft. SZA"},
        deadline(clock),
    )

    assert data["primary_artist"] == "Kendrick Lamar"
    assert data["featured"] == ["SZA"]
    assert data["confidence"] == 0.9
    assert data["source"] == "deterministic"


async def test_title_artist_beats_a_reupload_channel(clock: FakeClock) -> None:
    subject, _ = resolver()

    data = await subject.resolve(
        {
            "title": "Billie Eilish - Ocean Eyes (feat. Finneas) (prod. Finneas)",
            "uploader": "Nightcore Vibes",
        },
        deadline(clock),
    )

    assert data["primary_artist"] == "Billie Eilish"
    assert data["featured"] == ["Finneas"]
    assert data["producers"] == ["Finneas"]
    assert data["confidence"] == 0.7


async def test_uploader_is_used_only_when_it_is_not_a_channel(clock: FakeClock) -> None:
    subject, _ = resolver()

    person = await subject.resolve({"title": "рассвет", "uploader": "Psychosis"}, deadline(clock))
    channel = await subject.resolve(
        {"title": "рассвет", "uploader": "Chill Beats Radio"}, deadline(clock)
    )

    assert (person["primary_artist"], person["confidence"]) == ("Psychosis", 0.5)
    assert (channel["primary_artist"], channel["confidence"]) == (None, 0.3)


async def test_confident_answers_never_call_the_llm(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    subject, _ = resolver(refiner)

    await subject.resolve({"title": "x", "metadata_artist": "Artist"}, deadline(clock))

    assert refiner.calls == []


async def test_gray_zone_is_refined_by_a_grounded_llm_answer(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(llm_reply(featured=["SZA"]))
    subject, counters = resolver(refiner)

    data = await subject.resolve(
        {"title": "Kendrick Lamar & SZA - luther", "uploader": "TDE"}, deadline(clock)
    )

    assert data["primary_artist"] == "Kendrick Lamar"
    assert data["featured"] == ["SZA"]
    assert data["source"] == "llm"
    assert counters.value("rpc_answers_total", method="resolve_artist", source="llm") == 1
    assert refiner.calls[0][1] is RESOLVE_SCHEMA


async def test_names_outside_the_input_are_rejected(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(llm_reply(primary_artist="Drake"))
    subject, counters = resolver(refiner)

    data = await subject.resolve({"title": "Kendrick Lamar - luther"}, deadline(clock))

    assert refiner.ungrounded == 1
    assert data["primary_artist"] == "Kendrick Lamar"
    assert data["source"] == "deterministic"
    assert counters.value("rpc_llm_fallbacks_total", method="resolve_artist") == 1


async def test_an_empty_llm_answer_does_not_erase_the_parsed_artist(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(llm_reply(primary_artist=None, featured=[], confidence=0.2))
    subject, counters = resolver(refiner)

    data = await subject.resolve(
        {"title": "Kendrick Lamar - HUMBLE.", "uploader": "rap hits archive"}, deadline(clock)
    )

    assert refiner.ungrounded == 1
    assert data["primary_artist"] == "Kendrick Lamar"
    assert data["source"] == "deterministic"
    assert counters.value("rpc_llm_fallbacks_total", method="resolve_artist") == 1


async def test_an_empty_llm_answer_is_kept_when_the_parser_found_nobody(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(llm_reply(primary_artist=None, featured=[], confidence=0.2))
    subject, _ = resolver(refiner)

    data = await subject.resolve(
        {"title": "рассвет", "uploader": "Chill Beats Radio"}, deadline(clock)
    )

    assert refiner.ungrounded == 0
    assert (data["primary_artist"], data["source"]) == (None, "llm")


async def test_a_name_spanning_two_fields_is_ungrounded(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(llm_reply(primary_artist="Drake Music", featured=[]))
    subject, _ = resolver(refiner)

    data = await subject.resolve(
        {"title": "Song by Drake", "uploader": "Music Nation"}, deadline(clock)
    )

    assert refiner.ungrounded == 1
    assert data["source"] == "deterministic"


async def test_a_transliterated_name_is_ungrounded(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(
        llm_reply(
            primary_artist="Земфира",
            featured=[],
            album={"title": "ИСКАЛА", "year": None, "primary_artist": None},
        )
    )
    subject, _ = resolver(refiner)

    data = await subject.resolve(
        {"title": "Zemfira - Iskala", "uploader": "ru hits channel"}, deadline(clock)
    )

    assert refiner.ungrounded == 1
    assert (data["primary_artist"], data["album"], data["source"]) == (
        "Zemfira",
        None,
        "deterministic",
    )


async def test_llm_names_are_published_as_written_in_the_input(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(
        llm_reply(
            primary_artist="KENDRICK LAMAR!!",
            featured=["sza"],
            album={"title": "gnx", "year": 2024, "primary_artist": "kendrick  lamar"},
        )
    )
    subject, _ = resolver(refiner)

    data = await subject.resolve(
        {"title": "Kendrick Lamar & SZA - luther", "description": "from the album GNX"},
        deadline(clock),
    )

    assert refiner.ungrounded == 0
    assert data["primary_artist"] == "Kendrick Lamar"
    assert data["featured"] == ["SZA"]
    assert data["album"] == {"title": "GNX", "year": 2024, "primary_artist": "Kendrick Lamar"}
    assert data["source"] == "llm"


async def test_album_title_must_be_grounded_too(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(
        llm_reply(
            featured=[],
            album={"title": "GNX", "year": 2024, "primary_artist": "Kendrick Lamar"},
        )
    )
    subject, _ = resolver(refiner)

    grounded = await subject.resolve(
        {"title": "Kendrick Lamar - luther", "description": "from the album GNX"}, deadline(clock)
    )

    assert grounded["album"] == {"title": "GNX", "year": 2024, "primary_artist": "Kendrick Lamar"}


async def test_implausible_album_year_discards_the_llm_answer(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(
        llm_reply(featured=[], album={"title": "GNX", "year": 3024, "primary_artist": None})
    )
    subject, _ = resolver(refiner)

    data = await subject.resolve(
        {"title": "Kendrick Lamar - luther", "description": "GNX"}, deadline(clock)
    )

    assert data["source"] == "deterministic"


async def test_exhausted_chain_answers_deterministically(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(None)
    subject, _ = resolver(refiner)

    data = await subject.resolve({"title": "Kendrick Lamar - luther"}, deadline(clock))

    assert data["source"] == "deterministic"
    assert len(refiner.calls) == 1


async def test_short_deadline_skips_the_llm(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    subject, counters = resolver(refiner)

    data = await subject.resolve({"title": "Kendrick Lamar - luther"}, deadline(clock, 3.4))

    assert refiner.calls == []
    assert data["source"] == "deterministic"
    assert counters.value("llm_skipped_total", method="resolve_artist", why="short_deadline") == 1


async def test_without_a_refiner_the_answer_is_deterministic(clock: FakeClock) -> None:
    subject, _ = resolver(None)

    data = await subject.resolve({"title": "Kendrick Lamar - luther"}, deadline(clock))

    assert data["source"] == "deterministic"


@pytest.mark.parametrize(
    "request_",
    [
        {},
        {"title": 5},
        {"title": ""},
        {"title": "x", "uploader": 3},
        {"title": "x", "duration_ms": True},
        {"title": "x", "description": "d" * 4001},
    ],
)
async def test_malformed_requests_are_invalid(
    clock: FakeClock, request_: Mapping[str, object]
) -> None:
    subject, _ = resolver()

    with pytest.raises(PermanentFailure) as failure:
        await subject.resolve(request_, deadline(clock))

    assert failure.value.reason is Reason.INVALID_REQUEST


async def test_answers_match_the_reply_schema(clock: FakeClock, contract: Contract) -> None:
    refiner = FakeRefiner()
    refiner.reply_with(None)
    refiner.reply_with(
        llm_reply(album={"title": "GNX", "year": None, "primary_artist": None}, featured=[])
    )
    subject, _ = resolver(refiner)

    deterministic = await subject.resolve({"title": "рассвет"}, deadline(clock))
    refined = await subject.resolve(
        {"title": "Kendrick Lamar - luther", "description": "GNX"}, deadline(clock)
    )

    for data in (deterministic, refined):
        assert contract.validate(REPLY, {"ok": True, "data": dict(data)}) == []
    assert refined["album"] == {"title": "GNX"}


async def test_an_expired_call_is_a_deadline_failure(clock: FakeClock) -> None:
    subject, _ = resolver()

    with pytest.raises(TransientFailure) as failure:
        await subject.resolve({"title": "x"}, deadline(clock, 0.5))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED


def test_the_llm_schema_accepts_its_own_example() -> None:
    assert validate(llm_reply(), RESOLVE_SCHEMA) == []
    assert validate(llm_reply(confidence=True), RESOLVE_SCHEMA) != []
