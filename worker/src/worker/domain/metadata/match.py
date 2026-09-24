from __future__ import annotations

import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from difflib import SequenceMatcher

from worker.domain.deadline import Deadline
from worker.domain.metadata import fields
from worker.domain.metadata.normalize import parse_title, version_markers, words
from worker.domain.outcome import PermanentFailure, Reason
from worker.domain.ports import Refiner
from worker.observability.counters import Counters

RPC_MARGIN_S = 0.5
LLM_MIN_REMAINING_S = 3.0
ARTIST_WEIGHT = 0.45
TITLE_WEIGHT = 0.45
DURATION_WEIGHT = 0.1
UNKNOWN_DURATION_SCORE = 0.5
DURATION_TOLERANCE_S = 4.0
DURATION_LIMIT_S = 25.0
DECISIVE_SCORE = 0.85
DECISIVE_MARGIN = 0.15
NO_MATCH_BELOW = 0.5
TIE_MARGIN = 0.05
LLM_CANDIDATES = 10
MAX_CANDIDATES = 50
MAX_CANDIDATE_ID = 0xFFFFFFFF
METHOD = "match_track"

MATCH_SCHEMA: Mapping[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["match_id", "confidence"],
    "properties": {
        "match_id": {"type": ["integer", "null"]},
        "confidence": {"type": "number"},
    },
}

log = logging.getLogger(__name__)


class TrackMatcher:
    def __init__(self, refiner: Refiner | None, counters: Counters) -> None:
        self._refiner = refiner
        self._counters = counters

    async def match(
        self, request: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        deadline.minus(RPC_MARGIN_S).check(METHOD)
        target, candidates = parse_request(request)
        ranked = rank(target, candidates)
        verdict = await self._refined(target, ranked, deadline)
        self._counters.inc("rpc_answers_total", method=METHOD, source=verdict.source)
        return verdict.to_wire()

    async def _refined(self, target: Track, ranked: list[Scored], deadline: Deadline) -> Verdict:
        verdict = decide(ranked)
        if self._refiner is None or not is_borderline(ranked):
            return verdict
        if deadline.minus(RPC_MARGIN_S).remaining() < LLM_MIN_REMAINING_S:
            self._counters.inc("llm_skipped_total", method=METHOD, why="short_deadline")
            return verdict
        shortlist = ranked[:LLM_CANDIDATES]
        allowed = {scored.candidate.id for scored in shortlist}
        reply = await self._refiner.refine(
            prompt(target, shortlist),
            MATCH_SCHEMA,
            lambda candidate: read_reply(candidate, allowed) is not None,
            deadline,
        )
        answer = read_reply(reply, allowed) if reply is not None else None
        if answer is None:
            self._counters.inc("rpc_llm_fallbacks_total", method=METHOD)
            log.info("match_track answers deterministically after the llm chain")
            return verdict
        return answer


@dataclass(frozen=True)
class Track:
    artist: str
    title: str
    duration_s: float | None

    @property
    def song(self) -> str:
        return parse_title(self.title).song or self.title

    @property
    def artists(self) -> tuple[str, ...]:
        titled = parse_title(self.title).artist
        return (self.artist, titled) if titled else (self.artist,)


@dataclass(frozen=True)
class Candidate:
    id: int
    track: Track


@dataclass(frozen=True)
class Scored:
    candidate: Candidate
    score: float


@dataclass(frozen=True)
class Verdict:
    match_id: int | None
    confidence: float
    source: str

    def to_wire(self) -> dict[str, object]:
        return {
            "match_id": self.match_id,
            "confidence": round(self.confidence, 4),
            "source": self.source,
        }


def parse_request(request: Mapping[str, object]) -> tuple[Track, list[Candidate]]:
    target = parse_track(fields.required_mapping(request, "target"))
    raw_candidates = fields.required_list(request, "candidates")
    if not 1 <= len(raw_candidates) <= MAX_CANDIDATES:
        raise PermanentFailure(
            Reason.INVALID_REQUEST, f"candidates must hold 1..{MAX_CANDIDATES} items"
        )
    candidates: list[Candidate] = []
    for raw in raw_candidates:
        if not isinstance(raw, Mapping):
            raise PermanentFailure(Reason.INVALID_REQUEST, "candidate must be an object")
        candidates.append(Candidate(parse_candidate_id(raw), parse_track(raw)))
    return target, candidates


def parse_candidate_id(raw: Mapping[str, object]) -> int:
    candidate_id = fields.required_int(raw, "id")
    if not 0 <= candidate_id <= MAX_CANDIDATE_ID:
        raise PermanentFailure(Reason.INVALID_REQUEST, f"id {candidate_id} is not a u32")
    return candidate_id


def parse_track(raw: Mapping[str, object]) -> Track:
    return Track(
        artist=fields.required_text(raw, "artist"),
        title=fields.required_text(raw, "title"),
        duration_s=fields.optional_number(raw, "duration_sec"),
    )


def rank(target: Track, candidates: Sequence[Candidate]) -> list[Scored]:
    wanted = version_markers(target.title)
    eligible = [
        Scored(candidate, score(target, candidate.track))
        for candidate in candidates
        if version_markers(candidate.track.title) == wanted
    ]
    return sorted(eligible, key=lambda scored: -scored.score)


def score(target: Track, candidate: Track) -> float:
    artist = max(
        token_set(wanted, offered) for wanted in target.artists for offered in candidate.artists
    )
    title = token_set(target.song, candidate.song)
    duration = duration_closeness(target.duration_s, candidate.duration_s)
    return ARTIST_WEIGHT * artist + TITLE_WEIGHT * title + DURATION_WEIGHT * duration


def decide(ranked: list[Scored]) -> Verdict:
    if not ranked:
        return Verdict(None, 0.0, "deterministic")
    best = ranked[0].score
    if best < NO_MATCH_BELOW or margin(ranked) < TIE_MARGIN:
        return Verdict(None, best, "deterministic")
    return Verdict(ranked[0].candidate.id, best, "deterministic")


def is_borderline(ranked: list[Scored]) -> bool:
    if not ranked or ranked[0].score < NO_MATCH_BELOW:
        return False
    return not (ranked[0].score >= DECISIVE_SCORE and margin(ranked) >= DECISIVE_MARGIN)


def margin(ranked: list[Scored]) -> float:
    runner_up = ranked[1].score if len(ranked) > 1 else 0.0
    return ranked[0].score - runner_up


def token_set(left: str, right: str) -> float:
    ours, theirs = set(words(left)), set(words(right))
    if not ours or not theirs:
        return 0.0
    shared = " ".join(sorted(ours & theirs))
    with_ours = " ".join(filter(None, (shared, " ".join(sorted(ours - theirs)))))
    with_theirs = " ".join(filter(None, (shared, " ".join(sorted(theirs - ours)))))
    pairs = ((with_ours, with_theirs), (shared, with_ours), (shared, with_theirs))
    return max(SequenceMatcher(None, a, b).ratio() for a, b in pairs if a and b)


def duration_closeness(target_s: float | None, candidate_s: float | None) -> float:
    if target_s is None or candidate_s is None:
        return UNKNOWN_DURATION_SCORE
    difference = abs(target_s - candidate_s)
    if difference <= DURATION_TOLERANCE_S:
        return 1.0
    if difference >= DURATION_LIMIT_S:
        return 0.0
    return 1.0 - (difference - DURATION_TOLERANCE_S) / (DURATION_LIMIT_S - DURATION_TOLERANCE_S)


def read_reply(reply: Mapping[str, object], allowed: set[int]) -> Verdict | None:
    try:
        match_id = fields.reply_int(reply.get("match_id"))
        confidence = fields.reply_share(reply.get("confidence"))
    except fields.UnreadableReply as error:
        log.info("match_track reply rejected", extra={"why": str(error)})
        return None
    if match_id is not None and match_id not in allowed:
        log.info("match_track reply names a candidate outside the shortlist")
        return None
    return Verdict(match_id, confidence, "llm")


def prompt(target: Track, shortlist: Sequence[Scored]) -> str:
    lines = [
        "Pick the candidate that is the same recording as the target, or null if none is.",
        "A remix, live, acoustic, instrumental, sped up or slowed version is a different",
        "recording. Candidate artists are often uploader names, not the performer.",
        f"target: artist={target.artist!r} title={target.title!r}",
        "candidates:",
        *(candidate_line(scored.candidate) for scored in shortlist),
    ]
    return "\n".join(lines)


def candidate_line(candidate: Candidate) -> str:
    track = candidate.track
    return (
        f"- id={candidate.id} artist={track.artist!r} title={track.title!r} "
        f"duration_sec={track.duration_s}"
    )
