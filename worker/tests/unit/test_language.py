from __future__ import annotations

import pytest

from tests.fakes.engines import FakeEngines
from worker.domain import language
from worker.domain.deadline import Deadline
from worker.domain.ports import LanguageGuess

RU_LONG = [
    "Я иду по улице ночной и снова думаю о тебе",
    "Город спит, а я один стою под фонарём",
    "Мы обещали друг другу больше не прощаться",
    "Всё, что было между нами, унесло рекой",
    "И только ветер знает, где теперь твой дом",
    "Я пел тебе о лете, ты молчала в ответ",
    "Снова утро, снова кофе и пустой вокзал",
]
RU_SHORT = ["Ла-ла-ла", "Эй, эй", "О-о-о"]


def guesses(*pairs: tuple[str, float]) -> list[LanguageGuess]:
    return [LanguageGuess(code, prob) for code, prob in pairs]


async def detect(
    engines: FakeEngines, text: str, hint: str | None = None
) -> language.LanguageDetection:
    return await language.detect(text, hint, engines, Deadline.after(30))


@pytest.mark.parametrize(
    ("code", "internal", "wire"),
    [
        ("ru", "ru", "ru"),
        ("EN", "en", "en"),
        ("tl", "fil", "tl"),
        ("fil", "fil", "tl"),
        ("yue", "yue", "zh"),
        ("ceb", "ceb", None),
        ("war", "war", None),
        ("sh", "hbs", None),
        ("", None, None),
        (None, None, None),
    ],
)
def test_iso_map(code: str | None, internal: str | None, wire: str | None) -> None:
    assert language.to_internal(code) == internal
    assert language.to_wire(code) == wire


def test_cyrillic_languages_share_a_group() -> None:
    assert language.language_group("ru") == language.language_group("uk")
    assert language.language_group("bg") == language.language_group("sr")
    assert language.language_group("en") != language.language_group("de")


def test_lines_drop_timestamps_markers_and_blanks() -> None:
    text = "[Chorus]\r\n[00:12.34] Hello there\n\n(Verse 2)\n  spaced   out  \n[01:02]"

    assert language.lines_for_detection(text) == ["Hello there", "spaced out"]


@pytest.mark.parametrize(
    ("text", "code"),
    [
        ("사랑해 너를 보고 싶어", "ko"),
        ("きみのことがすきだよ", "ja"),
        ("東京の夜はさびしいね", "ja"),
        ("我爱你中国", "zh"),
        ("Καλημέρα κόσμε", "el"),
        ("שלום עולם", "he"),
        ("สวัสดีครับ", "th"),
        ("مرحبا بالعالم", "ar"),
        ("hello world", None),
        ("12345 !!!", None),
    ],
)
def test_script_fallback(text: str, code: str | None) -> None:
    assert language.script_fallback(text, {}) == code


def test_cyrillic_fallback_takes_best_cyrillic_score() -> None:
    scores = {"ru": 0.2, "uk": 0.35, "bg": 0.1}

    assert language.script_fallback("Привіт, як справи", scores) == "uk"
    assert language.script_fallback("Привіт, як справи", {}) is None


async def test_confident_track_language(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ru", 0.95), ("uk", 0.03))

    detection = await detect(engines, "\n".join(RU_LONG))

    assert detection.track == "ru"
    assert detection.prob == pytest.approx(0.95)
    assert detection.lines == ["ru"] * len(RU_LONG)


async def test_short_lines_mislabelled_bg_inherit_track_language(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ru", 0.9))
    for line in RU_SHORT:
        engines.languages[line] = guesses(("bg", 0.85), ("ru", 0.1))
    lines = RU_LONG + RU_SHORT
    assert len(RU_SHORT) / len(lines) >= 0.3

    detection = await detect(engines, "\n".join(lines))

    assert detection.track == "ru"
    assert set(detection.lines) == {"ru"}


async def test_long_line_of_same_group_inherits(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ru", 0.9))
    uk_line = "Я співаю тобі цю пісню до самого ранку"
    engines.languages[uk_line] = guesses(("uk", 0.95))

    detection = await detect(engines, "\n".join([*RU_LONG, uk_line]))

    assert detection.lines[-1] == "ru"


async def test_long_line_in_other_script_overrides(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ru", 0.9))
    en_line = "Baby I just want to hold you tonight"
    engines.languages[en_line] = guesses(("en", 0.93))

    detection = await detect(engines, "\n".join([*RU_LONG, en_line]))

    assert detection.track == "ru"
    assert detection.lines[-1] == "en"


async def test_uncertain_line_inherits(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ru", 0.9))
    en_line = "Baby I just want to hold you tonight"
    engines.languages[en_line] = guesses(("en", 0.7))

    detection = await detect(engines, "\n".join([*RU_LONG, en_line]))

    assert detection.lines[-1] == "ru"


async def test_line_indexes_follow_raw_lines(engines: FakeEngines) -> None:
    engines.default_language = guesses(("en", 0.95))
    text = "[Chorus]\nI walk alone along the empty road\n\nI walk alone along the silent sea"

    detection = await detect(engines, text)

    assert detection.lines == ["en", "en", "en", "en"]
    assert engines.calls[0][1]["lines"] == [
        "I walk alone along the empty road",
        "I walk alone along the silent sea",
    ]


async def test_mixed_text_returns_the_first_language(engines: FakeEngines) -> None:
    engines.default_language = guesses(("en", 0.95))
    ru = RU_LONG[:6]
    for line in ru:
        engines.languages[line] = guesses(("ru", 0.95))
    en = ["We are young and we are free tonight", "Nothing can stop us, we will run"]

    detection = await detect(engines, "\n".join(ru + en))

    assert detection.track == "ru"
    assert detection.lines[-2:] == ["en", "en"]


async def test_low_probability_falls_back_to_script(engines: FakeEngines) -> None:
    text = "사랑해 너를 보고 싶어 오늘 밤에 너와 함께"
    engines.default_language = guesses(("ja", 0.4), ("ko", 0.35))

    detection = await detect(engines, text)

    assert detection.track == "ko"


async def test_cyrillic_fallback_uses_weighted_scores(engines: FakeEngines) -> None:
    engines.default_language = guesses(("uk", 0.45), ("ru", 0.4))

    detection = await detect(engines, "\n".join(RU_LONG))

    assert detection.track == "uk"


async def test_too_few_characters_fall_back_to_script(engines: FakeEngines) -> None:
    engines.default_language = guesses(("de", 0.99))

    detection = await detect(engines, "Hallo du")

    assert detection.track is None


async def test_hint_has_priority(engines: FakeEngines) -> None:
    engines.default_language = guesses(("en", 0.99))

    detection = await detect(engines, "\n".join(RU_SHORT), hint="tl")

    assert detection.track == "fil"
    assert set(detection.lines) == {"fil"}


async def test_track_probability_belongs_to_the_hinted_language(engines: FakeEngines) -> None:
    engines.default_language = guesses(("zh", 0.85), ("ja", 0.1))

    detection = await detect(engines, "東京の夜空に星が輝いて\n君の名前を呼ぶ", hint="ja")

    assert detection.track == "ja"
    assert set(detection.lines) == {"ja"}
    assert detection.prob == pytest.approx(0.1)


@pytest.mark.parametrize(
    "separator",
    ["\r", "\N{LINE SEPARATOR}", "\N{PARAGRAPH SEPARATOR}", "\x85", "\v", "\f", "\r\n"],
)
def test_every_unicode_line_break_splits_lines(separator: str) -> None:
    assert language.split_lines(separator.join(["раз", "два", "три"])) == ["раз", "два", "три"]


@pytest.mark.parametrize(
    ("line", "marker"),
    [
        ("[Verse 1]", True),
        ("[Припев]", True),
        ("(Chorus)", True),
        ("(Chorus: Drake)", True),
        ("(Pre-Chorus)", True),
        ("(Куплет 2)", True),
        ("(Instrumental)", True),
        ("(x2)", True),
        ("（サビ）", True),
        ("(Oh baby)", False),
        ("(I want you back)", False),
        ("（ラララ）", False),
        ("(Hookah smoke)", False),
    ],
)
def test_only_section_words_in_round_brackets_make_a_marker(line: str, marker: bool) -> None:
    assert bool(language.SECTION_MARKER.fullmatch(line)) == marker


async def test_filipino_and_cantonese(engines: FakeEngines) -> None:
    engines.default_language = guesses(("tl", 0.97))
    tagalog = await detect(engines, "Mahal kita, ikaw lang ang aking mahal magpakailanman")
    engines.default_language = guesses(("yue", 0.9))
    cantonese = await detect(engines, "我哋一齊去睇海啦，今晚嘅月光好靚呀，你記唔記得")

    assert (tagalog.track, language.to_wire(tagalog.track)) == ("fil", "tl")
    assert (cantonese.track, language.to_wire(cantonese.track)) == ("yue", "zh")


async def test_cebuano_has_no_wire_code(engines: FakeEngines) -> None:
    engines.default_language = guesses(("ceb", 0.9))

    detection = await detect(engines, "Gihigugma tika sa tanan nakong kasingkasing")

    assert detection.track == "ceb"
    assert language.to_wire(detection.track) is None


async def test_text_without_lines_skips_the_engine(engines: FakeEngines) -> None:
    detection = await detect(engines, "[Intro]\n\n[00:01.00]")

    assert detection.track is None
    assert detection.prob == 0.0
    assert engines.calls == []
