from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass
from itertools import pairwise

from unidecode import unidecode

SEPARATOR = re.compile(r"\s+[-–—−]\s+|\s+[|｜]\s+")
BRACKETED = re.compile(r"[(\[{]([^()\[\]{}]*)[)\]}]")
FEATURED_INLINE = re.compile(r"\b(?:feat|ft|featuring)\b\.?\s*(.+)$", re.IGNORECASE)
FEATURED_NOTE = re.compile(r"^(?:feat|ft|featuring|with)\b\.?\s*(.+)$", re.IGNORECASE)
PRODUCED = re.compile(r"\b(?:prod|produced)\b\.?\s*(?:by\b)?\s*[.:]?\s*(.+)$", re.IGNORECASE)
REMIX = re.compile(r"^(.+?)\s+(?:remix|bootleg|flip|edit|rework|mashup|vip)\b", re.IGNORECASE)
HANDLE = re.compile(r"[@#]\S+")
NAME_SPLIT = re.compile(r"\s*(?:,|&|\bx\b|\band\b|\+)\s*", re.IGNORECASE)
WHITESPACE = re.compile(r"\s+")
WORD = re.compile(r"\w+")

NOISE_PHRASES = frozenset(
    {
        "official video",
        "official audio",
        "official music video",
        "official lyric video",
        "official visualizer",
        "music video",
        "lyric video",
        "lyrics",
        "audio",
        "video",
        "visualizer",
        "hq",
        "hd",
        "4k",
        "free dl",
        "free download",
        "free",
        "download",
        "out now",
        "premiere",
        "exclusive",
        "explicit",
        "clean",
        "extended",
        "full version",
        "original mix",
        "radio edit",
        "master",
        "remastered",
        "cover art",
        "snippet",
        "teaser",
        "preview",
        "demo",
        "unreleased",
        "leak",
        "leaked",
        "prod",
    }
)

VERSION_PHRASES = frozenset(
    {
        "slowed",
        "slowed reverb",
        "slowed + reverb",
        "sped up",
        "speed up",
        "nightcore",
        "daycore",
        "reverb",
        "8d",
        "bass boosted",
        "bassboosted",
        "pitched",
        "chopped",
        "screwed",
        "instrumental",
        "acapella",
        "live",
        "acoustic",
        "remix",
    }
)
VERSION_WORDS = frozenset({"slowed", "sped", "nightcore", "reverb", "bassboosted", "8d"})

REUPLOAD_MARKERS = (
    "nightcore",
    "vibes",
    "boost",
    "chill",
    "slowed",
    "sped",
    "type beat",
    "typebeat",
    "promo",
    "release",
    "records",
    "music",
    "channel",
    "archive",
    "uploads",
    "reupload",
    "daily",
    "hits",
    "mix",
    "radio",
    "network",
)

MARKER_WORDS = {
    "remix": "remix",
    "instrumental": "instrumental",
    "acoustic": "acoustic",
    "slowed": "slowed",
    "nightcore": "sped_up",
}
MARKER_PHRASES = {("sped", "up"): "sped_up", ("speed", "up"): "sped_up"}
BRACKET_ONLY_MARKERS = {"live": "live"}


@dataclass(frozen=True)
class TitleParts:
    artist: str | None
    song: str
    featured: tuple[str, ...]
    producers: tuple[str, ...]
    remixers: tuple[str, ...]
    versions: tuple[str, ...]


@dataclass(frozen=True)
class Credits:
    main: str | None
    featured: tuple[str, ...]


def parse_title(title: str) -> TitleParts:
    notes, versions, bare = strip_brackets(tidy(HANDLE.sub(" ", title)))
    artist_part, song = split_artist_and_song(bare)
    featured: list[str] = []
    producers: list[str] = []
    remixers: list[str] = []
    for note in notes:
        if found := FEATURED_NOTE.search(note):
            featured.extend(names(found.group(1)))
        elif found := PRODUCED.search(note):
            producers.extend(names(found.group(1)))
        elif found := REMIX.search(note):
            remixers.extend(names(found.group(1)))
    artist_part, inline_featured = cut(FEATURED_INLINE, artist_part)
    song, song_featured = cut(FEATURED_INLINE, song)
    song, song_producers = cut(PRODUCED, song)
    return TitleParts(
        artist=artist_part or None,
        song=song,
        featured=unique([*inline_featured, *song_featured, *featured]),
        producers=unique([*song_producers, *producers]),
        remixers=unique(remixers),
        versions=versions,
    )


def split_credits(artist: str) -> Credits:
    main, featured = cut(FEATURED_INLINE, tidy(artist))
    return Credits(main=main or None, featured=featured)


def is_reupload_channel(uploader: str) -> bool:
    folded = uploader.casefold()
    return any(marker in folded for marker in REUPLOAD_MARKERS)


def fold(text: str) -> str:
    return " ".join(words(text))


def words(text: str) -> list[str]:
    plain = unidecode(unicodedata.normalize("NFKC", text).casefold()).casefold()
    return WORD.findall(plain)


def version_markers(title: str) -> frozenset[str]:
    found: set[str] = set()
    title_words = words(title)
    found.update(MARKER_WORDS[word] for word in title_words if word in MARKER_WORDS)
    found.update(
        marker
        for (first, second), marker in MARKER_PHRASES.items()
        if (first, second) in pairwise(title_words)
    )
    for note in BRACKETED.findall(title):
        found.update(
            BRACKET_ONLY_MARKERS[word] for word in words(note) if word in BRACKET_ONLY_MARKERS
        )
    return frozenset(found)


def find_name(text: str, name: str) -> str | None:
    needle = [token.casefold() for token in WORD.findall(unicodedata.normalize("NFKC", name))]
    if not needle:
        return None
    plain = unicodedata.normalize("NFKC", text)
    spans = list(WORD.finditer(plain))
    width = len(needle)
    for start in range(len(spans) - width + 1):
        window = spans[start : start + width]
        if [span.group().casefold() for span in window] == needle:
            return plain[window[0].start() : window[-1].end()]
    return None


def tidy(text: str) -> str:
    kept = "".join(
        character
        for character in unicodedata.normalize("NFKC", text)
        if not unicodedata.category(character).startswith(("So", "Cn"))
    )
    return WHITESPACE.sub(" ", kept).strip(" -–—|·•,")


def strip_brackets(text: str) -> tuple[list[str], tuple[str, ...], str]:
    notes: list[str] = []
    versions: list[str] = []

    def classify(match: re.Match[str]) -> str:
        inner = tidy(match.group(1))
        folded = inner.casefold().strip(". ")
        if not inner or folded in NOISE_PHRASES:
            return " "
        if folded in VERSION_PHRASES or VERSION_WORDS & set(re.split(r"[\s+]+", folded)):
            versions.append(folded)
            return " "
        notes.append(inner)
        return " "

    bare = tidy(BRACKETED.sub(classify, text))
    return notes, unique(versions), bare


def split_artist_and_song(text: str) -> tuple[str, str]:
    parts = [part for part in SEPARATOR.split(text) if part.strip()]
    if len(parts) < 2:
        return "", tidy(text)
    return tidy(parts[0]), tidy(" ".join(parts[1:]))


def cut(pattern: re.Pattern[str], text: str) -> tuple[str, tuple[str, ...]]:
    found = pattern.search(text)
    if found is None:
        return text, ()
    return tidy(text[: found.start()]), tuple(names(found.group(1)))


def names(raw: str) -> list[str]:
    return [name for name in (tidy(part) for part in NAME_SPLIT.split(raw)) if name]


def unique(items: list[str] | tuple[str, ...]) -> tuple[str, ...]:
    seen: dict[str, str] = {}
    for item in items:
        seen.setdefault(item.casefold(), item)
    return tuple(seen.values())
