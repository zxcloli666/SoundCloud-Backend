from __future__ import annotations

import re
import unicodedata
from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from types import MappingProxyType

from worker.domain.deadline import Deadline
from worker.domain.ports import Engines, LanguageGuess

ALIGNER_LANGUAGES = frozenset({"zh", "en", "yue", "fr", "de", "it", "ja", "ko", "pt", "ru", "es"})

ASR_LANGUAGES = frozenset(
    {
        "zh",
        "en",
        "yue",
        "ar",
        "de",
        "fr",
        "es",
        "pt",
        "id",
        "it",
        "ko",
        "ru",
        "th",
        "vi",
        "ja",
        "tr",
        "hi",
        "ms",
        "nl",
        "sv",
        "da",
        "fi",
        "pl",
        "cs",
        "fil",
        "fa",
        "el",
        "hu",
        "mk",
        "ro",
    }
)

CJK_LANGUAGES = frozenset({"zh", "yue", "ja", "ko"})

CYRILLIC_GROUP = frozenset({"ru", "uk", "be", "bg", "mk", "sr"})

SCRIPT_FALLBACK: Mapping[str, str] = MappingProxyType(
    {
        "Hangul": "ko",
        "Hiragana": "ja",
        "Katakana": "ja",
        "Han": "zh",
        "Greek": "el",
        "Hebrew": "he",
        "Thai": "th",
        "Arabic": "ar",
    }
)

MIN_TRACK_PROB = 0.6
MIN_TRACK_CHARS = 20
MIN_LINE_PROB = 0.8
MIN_LINE_CHARS = 20

LINE_BREAK = re.compile(r"\r\n|[\n\r\v\f\x85\N{LINE SEPARATOR}\N{PARAGRAPH SEPARATOR}]")
LRC_TIMESTAMP = re.compile(r"\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?\]")
SECTION_WORDS = (
    r"pre-?chorus|chorus|verse|hook|bridge|intro|outro|refrain|interlude|instrumental|break"
    r"|solo|drop|skit|куплет|припев|бридж|вступление|проигрыш|интро|аутро|хук|サビ|間奏|副歌|후렴"
)
REPEAT_MARK = r"[x×хX]\s*\d{1,2}|\d{1,2}\s*[x×хX]"
SECTION_MARKER = re.compile(
    rf"[\[【][^\[\]【】]{{1,38}}[\]】]"
    rf"|[(（]\s*(?:(?:{SECTION_WORDS})(?![^\W\d_])[^()（）]{{0,30}}|(?:{REPEAT_MARK})\s*)[)）]",
    re.IGNORECASE,
)

INTERNAL_BY_LID: Mapping[str, str] = MappingProxyType({"tl": "fil", "sh": "hbs"})
WIRE_BY_INTERNAL: Mapping[str, str] = MappingProxyType({"fil": "tl", "yue": "zh"})


def to_internal(code: str | None) -> str | None:
    if not code:
        return None
    lowered = code.strip().lower()
    if not lowered:
        return None
    return INTERNAL_BY_LID.get(lowered, lowered)


def to_wire(code: str | None) -> str | None:
    internal = to_internal(code)
    if internal is None:
        return None
    wire = WIRE_BY_INTERNAL.get(internal, internal)
    return wire if len(wire) == 2 and wire.isalpha() else None


def language_group(code: str) -> str:
    return "cyrillic" if code in CYRILLIC_GROUP else code


@dataclass(frozen=True)
class LanguageDetection:
    track: str | None
    lines: Sequence[str | None]
    prob: float
    chars: int


def lines_for_detection(text: str) -> list[str]:
    return [line for _, line in indexed_lines(text)]


def script_fallback(text: str, cyrillic_scores: Mapping[str, float]) -> str | None:
    counts = script_counts(text)
    letters = sum(counts.values())
    if not letters:
        return None
    kana = counts.get("Hiragana", 0) + counts.get("Katakana", 0)
    if kana and (kana + counts.get("Han", 0)) * 2 >= letters:
        return "ja"
    script, count = counts.most_common(1)[0]
    if count * 2 < letters:
        return None
    if script == "Cyrillic":
        return best_cyrillic(cyrillic_scores)
    return SCRIPT_FALLBACK.get(script)


async def detect(
    text: str, hint: str | None, engines: Engines, deadline: Deadline
) -> LanguageDetection:
    lines = indexed_lines(text)
    guesses = await engines.detect_language([line for _, line in lines], deadline) if lines else []
    chars = sum(len(line) for _, line in lines)
    scores = weighted_scores([line for _, line in lines], guesses, chars)
    best, prob = max(scores.items(), key=lambda item: item[1], default=(None, 0.0))
    track = to_internal(hint)
    if track is None:
        track = track_language(best, prob, chars, lines, scores)
    per_line: list[str | None] = [track] * len(split_lines(text))
    text_script = dominant_script(" ".join(line for _, line in lines))
    for (index, line), line_guesses in zip(lines, guesses, strict=True):
        per_line[index] = line_language(line, line_guesses, track, text_script)
    track_prob = scores.get(track, 0.0) if track is not None else 0.0
    return LanguageDetection(track=track, lines=per_line, prob=track_prob, chars=chars)


def track_language(
    best: str | None,
    prob: float,
    chars: int,
    lines: Sequence[tuple[int, str]],
    scores: Mapping[str, float],
) -> str | None:
    if best is not None and prob >= MIN_TRACK_PROB and chars >= MIN_TRACK_CHARS:
        return best
    cyrillic = {code: scores.get(code, 0.0) for code in CYRILLIC_GROUP}
    return script_fallback(" ".join(line for _, line in lines), cyrillic)


def line_language(
    line: str,
    guesses: Sequence[LanguageGuess],
    track: str | None,
    text_script: str | None,
) -> str | None:
    if not guesses or len(line) < MIN_LINE_CHARS:
        return track
    top = max(guesses, key=lambda guess: guess.prob)
    code = to_internal(top.code)
    if code is None or top.prob < MIN_LINE_PROB or code == track:
        return track
    if track is None:
        return code
    if language_group(code) != language_group(track) or dominant_script(line) != text_script:
        return code
    return track


def weighted_scores(
    lines: Sequence[str], guesses: Sequence[Sequence[LanguageGuess]], chars: int
) -> dict[str, float]:
    scores: dict[str, float] = {}
    if not chars:
        return scores
    for line, line_guesses in zip(lines, guesses, strict=True):
        for guess in line_guesses:
            code = to_internal(guess.code)
            if code is not None:
                scores[code] = scores.get(code, 0.0) + len(line) * guess.prob / chars
    return scores


def best_cyrillic(scores: Mapping[str, float]) -> str | None:
    code, score = max(scores.items(), key=lambda item: item[1], default=(None, 0.0))
    return code if score > 0 else None


def indexed_lines(text: str) -> list[tuple[int, str]]:
    indexed: list[tuple[int, str]] = []
    for index, raw in enumerate(split_lines(text)):
        line = " ".join(LRC_TIMESTAMP.sub(" ", raw).split())
        if line and not SECTION_MARKER.fullmatch(line):
            indexed.append((index, line))
    return indexed


def split_lines(text: str) -> list[str]:
    return LINE_BREAK.split(text)


def dominant_script(text: str) -> str | None:
    counts = script_counts(text)
    return counts.most_common(1)[0][0] if counts else None


def script_counts(text: str) -> Counter[str]:
    counts: Counter[str] = Counter()
    for character in text:
        if character.isalpha():
            counts[script_of(character)] += 1
    return counts


def script_of(character: str) -> str:
    name = unicodedata.name(character, "")
    if name.startswith("CJK "):
        return "Han"
    first = name.split(" ", 1)[0]
    return first.capitalize() if first else "Unknown"
