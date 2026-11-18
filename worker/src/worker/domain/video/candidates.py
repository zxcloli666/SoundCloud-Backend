from __future__ import annotations

import re
import unicodedata
from collections.abc import Sequence
from difflib import SequenceMatcher

from worker.domain.video.ytdlp import Found
from worker.settings import VideoSection

NEGATIVE_WORDS = (
    "live", "cover", "remix", "nightcore", "sped up", "slowed", "8d", "karaoke", "reverb",
    "instrumental", "mashup", "reaction", "tutorial", "bass boosted", "loop", "1 hour", "extended",
)  # fmt: skip
NON_WORD = re.compile(r"[^\w]+")


def pick(
    found: Sequence[Found],
    artist: str,
    title: str,
    duration_s: float,
    settings: VideoSection,
) -> list[Found]:
    ours = words(f"{artist} {title}")
    allowed = [
        item
        for item in found
        if plausible_duration(item.duration_s, duration_s, settings)
        and not unwanted(item.title, ours)
    ]
    allowed.sort(key=lambda item: -similarity(ours, words(item.title)))
    return allowed[: settings.candidates_per_search]


def plausible_duration(candidate_s: float | None, track_s: float, settings: VideoSection) -> bool:
    if candidate_s is None:
        return False
    if not settings.min_candidate_s <= candidate_s <= settings.max_candidate_s:
        return False
    ratio = candidate_s / track_s
    close = abs(candidate_s - track_s) <= settings.duration_slack_s
    return close or settings.duration_ratio_low <= ratio <= settings.duration_ratio_high


def unwanted(candidate_title: str, ours: str) -> bool:
    theirs = f" {words(candidate_title)} "
    mine = f" {ours} "
    return any(f" {word} " in theirs and f" {word} " not in mine for word in NEGATIVE_WORDS)


def similarity(ours: str, theirs: str) -> float:
    return SequenceMatcher(None, ours, theirs).ratio()


def words(text: str) -> str:
    plain = unicodedata.normalize("NFKC", text).casefold()
    return NON_WORD.sub(" ", plain).strip()
