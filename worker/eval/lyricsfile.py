from __future__ import annotations

import re
import unicodedata
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from itertools import pairwise
from typing import Any

import yaml

END_SLACK_MS = 200
NON_WORD = re.compile(r"[\W_]+")


class LyricsfileError(ValueError):
    pass


@dataclass(frozen=True)
class RefWord:
    line: int
    text: str
    start_ms: int
    end_ms: int


@dataclass(frozen=True)
class RefLine:
    text: str
    start_ms: int
    end_ms: int | None
    words: tuple[RefWord, ...]


@dataclass(frozen=True)
class Lyricsfile:
    title: str
    artist: str
    language: str | None
    duration_ms: int | None
    lines: tuple[RefLine, ...]
    plain: str
    dropped_word_lines: int

    @property
    def words(self) -> list[RefWord]:
        return [word for line in self.lines for word in line.words]


def parse(text: str) -> Lyricsfile:
    try:
        raw = yaml.safe_load(text)
    except yaml.YAMLError as error:
        raise LyricsfileError("lyricsfile is not valid yaml") from error
    if not isinstance(raw, dict) or not isinstance(raw.get("lines"), list):
        raise LyricsfileError("lyricsfile has no lines")
    metadata = raw.get("metadata")
    meta: Mapping[str, Any] = metadata if isinstance(metadata, dict) else {}
    items = [item for item in raw["lines"] if isinstance(item, dict) and str(item.get("text", ""))]
    lines: list[RefLine] = []
    dropped = 0
    for item in items:
        line, valid = parse_line(len(lines), item)
        lines.append(line)
        dropped += 0 if valid else 1
    if not lines:
        raise LyricsfileError("lyricsfile has no lines")
    duration = meta.get("duration_ms")
    language = meta.get("language")
    return Lyricsfile(
        title=str(meta.get("title") or ""),
        artist=str(meta.get("artist") or ""),
        language=str(language) if language else None,
        duration_ms=int(duration) if isinstance(duration, int | float) and duration > 0 else None,
        lines=tuple(lines),
        plain=str(raw.get("plain") or ""),
        dropped_word_lines=dropped,
    )


def parse_line(index: int, item: Mapping[str, Any]) -> tuple[RefLine, bool]:
    text = str(item["text"])
    start = item.get("start_ms")
    if not isinstance(start, int | float):
        raise LyricsfileError(f"line {index} has no start_ms")
    end = item.get("end_ms")
    end_ms = int(end) if isinstance(end, int | float) else None
    raw_words = item.get("words")
    if not isinstance(raw_words, list) or not raw_words:
        return RefLine(text, int(start), end_ms, ()), True
    pairs = [(str(word.get("text", "")), word.get("start_ms")) for word in raw_words]
    if not all(isinstance(moment, int | float) for _, moment in pairs):
        return RefLine(text, int(start), end_ms, ()), False
    words = [(word_text, int(moment)) for word_text, moment in pairs]
    if validate(text, int(start), end_ms, words) is not None:
        return RefLine(text, int(start), end_ms, ()), False
    return RefLine(text, int(start), end_ms, with_ends(index, words, int(start), end_ms)), True


def validate(
    text: str, start_ms: int, end_ms: int | None, words: Sequence[tuple[str, int]]
) -> str | None:
    starts = [moment for _, moment in words]
    if any(following <= current for current, following in pairwise(starts)):
        return "word starts do not increase"
    if starts and starts[0] < start_ms:
        return "word starts before its line"
    if end_ms is not None and starts and starts[-1] > end_ms + END_SLACK_MS:
        return "word starts after its line"
    if any(not normalize(word_text) for word_text, _ in words):
        return "word has no letters"
    if normalize("".join(word_text for word_text, _ in words)) != normalize(text):
        return "words differ from the line text"
    return None


def with_ends(
    line: int, words: Sequence[tuple[str, int]], start_ms: int, end_ms: int | None
) -> tuple[RefWord, ...]:
    closing = end_ms if end_ms is not None else words[-1][1]
    ends = [moment for _, moment in words[1:]] + [max(closing, words[-1][1])]
    return tuple(
        RefWord(line, text, moment, end) for (text, moment), end in zip(words, ends, strict=True)
    )


def normalize(text: str) -> str:
    return NON_WORD.sub("", unicodedata.normalize("NFKC", text).casefold())


def to_lrc(lyricsfile: Lyricsfile) -> str:
    return "\n".join(f"{timestamp(line.start_ms)}{line.text}" for line in lyricsfile.lines)


def timestamp(milliseconds: int) -> str:
    minutes, rest = divmod(milliseconds, 60000)
    return f"[{minutes:02d}:{rest // 1000:02d}.{rest % 1000:03d}]"


def words_to_json(words: Sequence[RefWord]) -> list[dict[str, object]]:
    return [
        {"line": word.line, "text": word.text, "start_ms": word.start_ms, "end_ms": word.end_ms}
        for word in words
    ]


def words_from_json(raw: Sequence[Mapping[str, Any]]) -> list[RefWord]:
    return [
        RefWord(int(item["line"]), str(item["text"]), int(item["start_ms"]), int(item["end_ms"]))
        for item in raw
    ]
