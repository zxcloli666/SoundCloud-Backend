from __future__ import annotations

import logging
from collections.abc import Mapping
from dataclasses import dataclass, replace

from worker.domain.deadline import Deadline
from worker.domain.metadata import fields
from worker.domain.metadata.normalize import (
    find_name,
    is_reupload_channel,
    parse_title,
    split_credits,
    tidy,
    unique,
)
from worker.domain.outcome import PermanentFailure, Reason
from worker.domain.ports import Refiner
from worker.observability.counters import Counters

MAX_DESCRIPTION_CHARS = 4000
RPC_MARGIN_S = 0.5
LLM_MIN_REMAINING_S = 3.0
REFINE_BELOW_CONFIDENCE = 0.8
METADATA_CONFIDENCE = 0.9
TITLE_CONFIDENCE = 0.7
UPLOADER_CONFIDENCE = 0.5
UNKNOWN_CONFIDENCE = 0.3
EARLIEST_YEAR = 1900
LATEST_YEAR = 2100
METHOD = "resolve_artist"

NULLABLE_TEXT: Mapping[str, object] = {"type": ["string", "null"]}
TEXT_LIST: Mapping[str, object] = {"type": "array", "items": {"type": "string"}}
RESOLVE_SCHEMA: Mapping[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["primary_artist", "featured", "producers", "remixers", "album", "confidence"],
    "properties": {
        "primary_artist": NULLABLE_TEXT,
        "featured": TEXT_LIST,
        "producers": TEXT_LIST,
        "remixers": TEXT_LIST,
        "album": {
            "anyOf": [
                {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["title", "year", "primary_artist"],
                    "properties": {
                        "title": {"type": "string"},
                        "year": {"type": ["integer", "null"]},
                        "primary_artist": NULLABLE_TEXT,
                    },
                },
                {"type": "null"},
            ]
        },
        "confidence": {"type": "number"},
    },
}

log = logging.getLogger(__name__)


class ArtistResolver:
    def __init__(self, refiner: Refiner | None, counters: Counters) -> None:
        self._refiner = refiner
        self._counters = counters

    async def resolve(
        self, request: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        deadline.minus(RPC_MARGIN_S).check(METHOD)
        track = TrackQuery.parse(request)
        answer = await self._refined(track, deterministic(track), deadline)
        self._counters.inc("rpc_answers_total", method=METHOD, source=answer.source)
        return answer.to_wire()

    async def _refined(
        self, track: TrackQuery, guess: Resolution, deadline: Deadline
    ) -> Resolution:
        if self._refiner is None or guess.confidence >= REFINE_BELOW_CONFIDENCE:
            return guess
        if deadline.minus(RPC_MARGIN_S).remaining() < LLM_MIN_REMAINING_S:
            self._counters.inc("llm_skipped_total", method=METHOD, why="short_deadline")
            return guess
        sources = track.sources()
        reply = await self._refiner.refine(
            prompt(track, guess),
            RESOLVE_SCHEMA,
            lambda candidate: grounded(read_reply(candidate), guess, sources) is not None,
            deadline,
        )
        answer = grounded(read_reply(reply), guess, sources) if reply is not None else None
        if answer is None:
            self._counters.inc("rpc_llm_fallbacks_total", method=METHOD)
            log.info("resolve_artist answers deterministically after the llm chain")
            return guess
        return answer


@dataclass(frozen=True)
class TrackQuery:
    title: str
    uploader: str
    metadata_artist: str
    isrc: str
    description: str
    duration_ms: float | None

    @classmethod
    def parse(cls, request: Mapping[str, object]) -> TrackQuery:
        title = fields.required_text(request, "title")
        if not title:
            raise PermanentFailure(Reason.INVALID_REQUEST, "title must not be empty")
        description = fields.optional_text(request, "description")
        if len(description) > MAX_DESCRIPTION_CHARS:
            raise PermanentFailure(
                Reason.INVALID_REQUEST, f"description is longer than {MAX_DESCRIPTION_CHARS}"
            )
        return cls(
            title=title,
            uploader=fields.optional_text(request, "uploader"),
            metadata_artist=fields.optional_text(request, "metadata_artist"),
            isrc=fields.optional_text(request, "isrc"),
            description=description,
            duration_ms=fields.optional_number(request, "duration_ms"),
        )

    def sources(self) -> tuple[str, ...]:
        return (self.title, self.uploader, self.metadata_artist, self.description)


@dataclass(frozen=True)
class Album:
    title: str
    year: int | None
    primary_artist: str | None

    def to_wire(self) -> dict[str, object]:
        wire: dict[str, object] = {"title": self.title}
        if self.year is not None:
            wire["year"] = self.year
        if self.primary_artist is not None:
            wire["primary_artist"] = self.primary_artist
        return wire

    def respelled(self, written: Mapping[str, str]) -> Album:
        return replace(
            self,
            title=written[self.title],
            primary_artist=respell(self.primary_artist, written),
        )


@dataclass(frozen=True)
class Resolution:
    primary_artist: str | None
    featured: tuple[str, ...]
    producers: tuple[str, ...]
    remixers: tuple[str, ...]
    album: Album | None
    confidence: float
    source: str

    def names(self) -> list[str]:
        named = [self.primary_artist, *self.featured, *self.producers, *self.remixers]
        if self.album is not None:
            named += [self.album.title, self.album.primary_artist]
        return [name for name in named if name is not None]

    def respelled(self, written: Mapping[str, str]) -> Resolution:
        return replace(
            self,
            primary_artist=respell(self.primary_artist, written),
            featured=unique([written[name] for name in self.featured]),
            producers=unique([written[name] for name in self.producers]),
            remixers=unique([written[name] for name in self.remixers]),
            album=self.album.respelled(written) if self.album else None,
        )

    def to_wire(self) -> dict[str, object]:
        return {
            "primary_artist": self.primary_artist,
            "featured": list(self.featured),
            "producers": list(self.producers),
            "remixers": list(self.remixers),
            "album": self.album.to_wire() if self.album else None,
            "confidence": round(self.confidence, 4),
            "source": self.source,
        }


def deterministic(track: TrackQuery) -> Resolution:
    parts = parse_title(track.title)
    credits = split_credits(track.metadata_artist)
    featured = list(parts.featured)
    primary: str | None
    if credits.main:
        primary, confidence = credits.main, METADATA_CONFIDENCE
        featured = [*credits.featured, *featured]
    elif parts.artist:
        primary, confidence = parts.artist, TITLE_CONFIDENCE
    elif tidy(track.uploader) and not is_reupload_channel(track.uploader):
        primary, confidence = tidy(track.uploader), UPLOADER_CONFIDENCE
    else:
        primary, confidence = None, UNKNOWN_CONFIDENCE
    primary_key = primary.casefold() if primary else None
    return Resolution(
        primary_artist=primary,
        featured=tuple(name for name in unique(featured) if name.casefold() != primary_key),
        producers=parts.producers,
        remixers=parts.remixers,
        album=None,
        confidence=confidence,
        source="deterministic",
    )


def read_reply(reply: Mapping[str, object]) -> Resolution | None:
    try:
        return Resolution(
            primary_artist=fields.reply_text(reply.get("primary_artist")),
            featured=unique(fields.reply_texts(reply.get("featured"))),
            producers=unique(fields.reply_texts(reply.get("producers"))),
            remixers=unique(fields.reply_texts(reply.get("remixers"))),
            album=read_album(reply.get("album")),
            confidence=fields.reply_share(reply.get("confidence")),
            source="llm",
        )
    except fields.UnreadableReply as error:
        log.info("resolve_artist reply rejected", extra={"why": str(error)})
        return None


def read_album(value: object) -> Album | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise fields.UnreadableReply("album must be an object or null")
    title = fields.reply_text(value.get("title"))
    year = fields.reply_int(value.get("year"))
    if title is None:
        raise fields.UnreadableReply("album without a title")
    if year is not None and not EARLIEST_YEAR <= year <= LATEST_YEAR:
        raise fields.UnreadableReply(f"album year {year} is implausible")
    return Album(title, year, fields.reply_text(value.get("primary_artist")))


def grounded(
    answer: Resolution | None, guess: Resolution, sources: tuple[str, ...]
) -> Resolution | None:
    if answer is None:
        return None
    if guess.primary_artist is not None and answer.primary_artist is None:
        return None
    written: dict[str, str] = {}
    for name in answer.names():
        found = spelling(name, sources)
        if found is None:
            return None
        written[name] = found
    return answer.respelled(written)


def spelling(name: str, sources: tuple[str, ...]) -> str | None:
    for source in sources:
        found = find_name(source, name)
        if found is not None:
            return found
    return None


def respell(name: str | None, written: Mapping[str, str]) -> str | None:
    return None if name is None else written[name]


def prompt(track: TrackQuery, guess: Resolution) -> str:
    duration = int(track.duration_ms) if track.duration_ms is not None else "-"
    lines = [
        "Identify the credited artists of a SoundCloud upload using only the metadata below.",
        "Uploaders are often re-upload channels; the real artist is then usually written in",
        "the title as 'Artist - Title'. Every name you return must appear verbatim in the",
        "metadata. Return null or empty lists when the metadata does not say.",
        f"title: {track.title}",
        f"uploader: {track.uploader or '-'}",
        f"metadata_artist: {track.metadata_artist or '-'}",
        f"isrc: {track.isrc or '-'}",
        f"duration_ms: {duration}",
        f"parser guess: primary_artist={guess.primary_artist!r} featured={list(guess.featured)}",
        f"description: {track.description or '-'}",
    ]
    return "\n".join(lines)
