from __future__ import annotations

import re
import unicodedata
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from functools import cache
from types import MappingProxyType
from typing import Any

import pykakasi
from uroman import Uroman

from worker.domain.language import CJK_LANGUAGES
from worker.domain.lyrics.text import Script

APOSTROPHES = "'’"
ROMAN_LETTERS = re.compile(r"[^a-z']+")
DIGIT = re.compile(r"\d")
ROMAN_SYLLABLE = re.compile(r"[^aeiou]*[aeiou]+(?:[^aeiou]+$)?")
DIGIT_NAMES = ("zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine")
SYLLABLE_KEYED = frozenset({"ko", "th"})
KAKASI_LOADER: Any = pykakasi.kakasi
CJK_RANGES = (
    (0x4E00, 0x9FFF),
    (0x3400, 0x4DBF),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B73F),
    (0x2B740, 0x2B81F),
    (0x2B820, 0x2CEAF),
    (0xF900, 0xFAFF),
    (0x2F800, 0x2FA1F),
)
HANGUL_SYLLABLES = (0xAC00, 0xD7A3)
KANA_AND_HANGUL_RANGES = (
    (0x3040, 0x30FF),
    (0x31F0, 0x31FF),
    (0xFF66, 0xFF9F),
    (0x1100, 0x11FF),
    (0x3130, 0x318F),
    HANGUL_SYLLABLES,
)

UROMAN_CODES: Mapping[str, str] = MappingProxyType(
    {
        "ar": "ara",
        "az": "aze",
        "be": "bel",
        "bg": "bul",
        "de": "deu",
        "el": "ell",
        "en": "eng",
        "fa": "fas",
        "he": "heb",
        "hi": "hin",
        "hy": "hye",
        "ka": "kat",
        "kk": "kaz",
        "ko": "kor",
        "mk": "mkd",
        "ru": "rus",
        "sr": "srp",
        "th": "tha",
        "tr": "tur",
        "uk": "ukr",
        "uz": "uzb",
        "vi": "vie",
        "yue": "yue",
        "zh": "zho",
    }
)


@dataclass(frozen=True)
class TokenizedLine:
    ordinal: int
    entry: int
    language: str | None
    tokens: tuple[str, ...]
    romanized: tuple[str, ...]
    units: int
    letters: int

    @property
    def cjk(self) -> bool:
        return is_cjk(self.language)

    @property
    def romanization_missing(self) -> int:
        return sum(1 for token in self.romanized if not token)


def tokenize_script(script: Script, languages: Sequence[str | None]) -> list[TokenizedLine]:
    lines: list[TokenizedLine] = []
    for ordinal, line in enumerate(script.sung):
        language = languages[line.source] if line.source < len(languages) else None
        tokens = tuple(tokenize(line.text, language))
        lines.append(
            TokenizedLine(
                ordinal=ordinal,
                entry=line.index,
                language=language,
                tokens=tokens,
                romanized=tuple(romanize(tokens, language)),
                units=tempo_units(tokens, language),
                letters=sum(1 for character in line.text if character.isalnum()),
            )
        )
    return lines


def tokenize(text: str, language: str | None) -> list[str]:
    if language == "ja":
        return clean_tokens(japanese_tagger().tagging(text).words)
    if language in ("zh", "yue"):
        return clean_tokens(split_cjk_characters(text))
    if language == "ko":
        return clean_tokens(text.split())
    return clean_tokens(words(text))


def tempo_units(tokens: Sequence[str], language: str | None) -> int:
    if language == "th":
        return len(roman_syllables("".join(tokens), language))
    if is_cjk(language):
        return sum(len(token) for token in tokens)
    return len(tokens)


def romanize(tokens: Sequence[str], language: str | None) -> list[str]:
    if language == "ja":
        return [roman_only(kana_reading(token)) for token in tokens]
    code = UROMAN_CODES.get(language or "")
    return [roman_only(uroman().romanize_string(token, lcode=code)) for token in tokens]


def similarity_key(text: str, language: str | None) -> str:
    if language == "th":
        return " ".join(roman_syllables(text, language))
    if language == "ko":
        tokens = clean_tokens(split_characters(text, is_hangul_syllable))
    else:
        tokens = tokenize(text, language)
    if is_cjk(language):
        return " ".join(fold(token) for token in romanize(tokens, language) if token)
    return " ".join(fold(token) for token in tokens)


def roman_syllables(text: str, language: str) -> list[str]:
    joined = "".join(clean_tokens(words(text)))
    code = UROMAN_CODES.get(language)
    return ROMAN_SYLLABLE.findall(roman_only(uroman().romanize_string(joined, lcode=code)))


def fold(text: str) -> str:
    decomposed = unicodedata.normalize("NFKD", text)
    return "".join(
        character for character in decomposed if not unicodedata.combining(character)
    ).lower()


def missing_romanization_share(lines: Sequence[TokenizedLine]) -> float:
    total = sum(len(line.tokens) for line in lines)
    if total == 0:
        return 0.0
    return sum(line.romanization_missing for line in lines) / total


def is_cjk(language: str | None) -> bool:
    return language in CJK_LANGUAGES


def clean_tokens(raw: Sequence[str]) -> list[str]:
    cleaned = ("".join(character for character in token if kept(character)) for token in raw)
    return [token for token in cleaned if token]


def kept(character: str) -> bool:
    return character in APOSTROPHES or word_character(character) or is_cjk_character(character)


def word_character(character: str) -> bool:
    return unicodedata.category(character)[0] in "LMN"


def words(text: str) -> list[str]:
    found: list[str] = []
    buffer: list[str] = []
    for character in text:
        if buffer and script_changes(buffer[-1], character):
            flush(found, buffer)
        if word_character(character) or (character in APOSTROPHES and buffer):
            buffer.append(character)
            continue
        flush(found, buffer)
    flush(found, buffer)
    return [word.rstrip(APOSTROPHES) for word in found]


def script_changes(previous: str, character: str) -> bool:
    if not word_character(character) or unicodedata.category(character)[0] == "M":
        return False
    return is_east_asian(previous) != is_east_asian(character)


def is_east_asian(character: str) -> bool:
    point = ord(character)
    return is_cjk_character(character) or any(
        low <= point <= high for low, high in KANA_AND_HANGUL_RANGES
    )


def split_cjk_characters(text: str) -> list[str]:
    return split_characters(text, is_cjk_character)


def split_characters(text: str, is_unit: Callable[[str], bool]) -> list[str]:
    tokens: list[str] = []
    buffer: list[str] = []
    for character in text:
        if is_unit(character):
            flush(tokens, buffer)
            tokens.append(character)
        elif character.isspace():
            flush(tokens, buffer)
        else:
            buffer.append(character)
    flush(tokens, buffer)
    return tokens


def flush(tokens: list[str], buffer: list[str]) -> None:
    if buffer:
        tokens.append("".join(buffer))
        buffer.clear()


def is_cjk_character(character: str) -> bool:
    point = ord(character)
    return any(low <= point <= high for low, high in CJK_RANGES)


def is_hangul_syllable(character: str) -> bool:
    return HANGUL_SYLLABLES[0] <= ord(character) <= HANGUL_SYLLABLES[1]


def kana_reading(token: str) -> str:
    return "".join(part["hepburn"] for part in kakasi().convert(token))


def roman_only(text: str) -> str:
    spelled = DIGIT.sub(digit_name, unicodedata.normalize("NFKD", text).lower())
    return ROMAN_LETTERS.sub("", spelled)


def digit_name(match: re.Match[str]) -> str:
    return DIGIT_NAMES[unicodedata.digit(match.group())]


@cache
def japanese_tagger() -> Any:
    import nagisa

    return nagisa


@cache
def uroman() -> Uroman:
    return Uroman()


@cache
def kakasi() -> Any:
    return KAKASI_LOADER()
