from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass
from typing import Literal

from worker.domain.language import LINE_BREAK, LRC_TIMESTAMP, SECTION_MARKER

MAX_REPEATS = 16

REPEAT_BRACKETED = re.compile(
    r"[\[(]\s*(?:[x×хX]\s*(?P<count>\d{1,2})|(?P<count2>\d{1,2})\s*[x×хX])\s*[\])]\s*$",
    re.IGNORECASE,
)
REPEAT_BARE = re.compile(
    r"(?:^|\s)(?:[x×хX]\s?(?P<count>\d{1,2})|(?P<count2>\d{1,2})\s?[x×хX])\s*$",
    re.IGNORECASE,
)
TRAILING_SEPARATORS = re.compile(r"[\s\-–—:,]+$")

Kind = Literal["sung", "marker"]


@dataclass(frozen=True)
class Line:
    index: int
    source: int
    text: str
    kind: Kind

    @property
    def sung(self) -> bool:
        return self.kind == "sung"


@dataclass(frozen=True)
class Script:
    entries: tuple[Line, ...]
    received_lines: int

    @property
    def sung(self) -> tuple[Line, ...]:
        return tuple(line for line in self.entries if line.sung)

    def lines_total(self, reference_lines_total: int) -> int:
        return len(self.sung) + max(0, reference_lines_total - self.received_lines)


def parse(reference_text: str) -> Script:
    entries: list[Line] = []
    received = 0
    for source, raw in enumerate(LINE_BREAK.split(reference_text)):
        if not clean(raw):
            continue
        received += 1
        text = clean(LRC_TIMESTAMP.sub(" ", raw))
        body, repeats = split_repeat(text)
        if SECTION_MARKER.fullmatch(text) or SECTION_MARKER.fullmatch(body) or not spoken(body):
            entries.append(Line(len(entries), source, text, "marker"))
            continue
        for _ in range(repeats):
            entries.append(Line(len(entries), source, body, "sung"))
    return Script(tuple(entries), received)


def clean(raw: str) -> str:
    return " ".join(unicodedata.normalize("NFKC", raw).split())


def split_repeat(text: str) -> tuple[str, int]:
    match = REPEAT_BRACKETED.search(text) or REPEAT_BARE.search(text)
    if match is None:
        return text, 1
    body = TRAILING_SEPARATORS.sub("", text[: match.start()])
    count = int(match.group("count") or match.group("count2"))
    return body, max(1, min(count, MAX_REPEATS))


def spoken(text: str) -> bool:
    return any(unicodedata.category(character)[0] in "LN" for character in text)
