from __future__ import annotations

import pytest

from tests.unit.lyrics.conftest import lines_of
from worker.domain.lyrics import tokens


def test_russian_words_keep_apostrophes_and_drop_punctuation() -> None:
    assert tokens.tokenize("Зашей мне глаза, — чтоб я не видел!", "ru") == [
        "Зашей",
        "мне",
        "глаза",
        "чтоб",
        "я",
        "не",
        "видел",
    ]
    assert tokens.tokenize("don't stop me now", "en") == ["don't", "stop", "me", "now"]


@pytest.mark.parametrize(
    ("text", "language", "words", "romanized"),
    [
        ("नमस्ते दुनिया", "hi", ["नमस्ते", "दुनिया"], ["namaste", "duniyaa"]),
        ("สวัสดีครับ", "th", ["สวัสดีครับ"], ["swatdiikrap"]),
        ("مَرْحَبًا", "ar", ["مَرْحَبًا"], ["marhaba"]),
        ("שָׁלוֹם עוֹלָם", "he", ["שָׁלוֹם", "עוֹלָם"], ["shalom", "'olam"]),
    ],
)
def test_combining_marks_stay_inside_their_word(
    text: str, language: str, words: list[str], romanized: list[str]
) -> None:
    assert tokens.tokenize(text, language) == words
    assert tokens.romanize(words, language) == romanized


def test_apostrophes_join_words_but_never_hang_off_them() -> None:
    assert tokens.tokenize("rock'n'roll, 'cause I'm dreamin'", "en") == [
        "rock'n'roll",
        "cause",
        "I'm",
        "dreamin",
    ]


def test_chinese_is_split_into_characters_without_split() -> None:
    assert tokens.tokenize("你好世界 hello", "zh") == ["你", "好", "世", "界", "hello"]
    assert tokens.tempo_units(["你", "好", "世", "界", "hello"], "zh") == 9


def test_japanese_uses_nagisa_words_and_kana_romanisation() -> None:
    words = tokens.tokenize("今日はいい天気です", "ja")
    assert words == ["今日", "は", "いい", "天気", "です"]
    romanized = tokens.romanize(words, "ja")
    assert romanized == ["kyou", "ha", "ii", "tenki", "desu"]


def test_korean_is_split_by_eojeol() -> None:
    assert tokens.tokenize("안녕하세요 여러분 사랑해요", "ko") == [
        "안녕하세요",
        "여러분",
        "사랑해요",
    ]
    assert tokens.tempo_units(["안녕하세요", "여러분"], "ko") == 8


@pytest.mark.parametrize(
    ("word", "language", "expected"),
    [
        ("привет", "ru", "privet"),
        ("გამარჯობა", "ka", "gamarjoba"),
        ("ሰላም", "am", "salaame"),
        ("hello", "en", "hello"),
        ("♪♪", None, ""),
    ],
)
def test_romanisation_is_lowercase_latin_or_empty(
    word: str, language: str | None, expected: str
) -> None:
    assert tokens.romanize([word], language) == [expected]


def test_missing_romanisation_share_counts_tokens() -> None:
    assert tokens.tokenize("♪ ♪ ♪", "ru") == []
    assert tokens.missing_romanization_share(lines_of(["слова"], "ru")) == 0.0
    hieroglyphs = lines_of(["𓀀 𓀁 слово"], "ru")
    assert tokens.missing_romanization_share(hieroglyphs) == pytest.approx(2 / 3)


@pytest.mark.parametrize("language", ["uk", "kk", "en", "ja", "zh", None])
def test_numbers_are_spelled_out_for_the_aligner(language: str | None) -> None:
    assert tokens.romanize(["1", "24", "2pac"], language) == ["one", "twofour", "twopac"]
    line = lines_of(["1, 2, 3, 4"], language)[0]
    assert line.romanized == ("one", "two", "three", "four")
    assert line.romanization_missing == 0


def test_thai_tempo_counts_syllables_not_phrases() -> None:
    line = lines_of(["ฉันรักเธอ มากมาย"], "th")[0]
    assert line.tokens == ("ฉันรักเธอ", "มากมาย")
    assert line.units == 5
    assert tokens.similarity_key("ฉันรักเธอมากมาย", "th") == "cha nra kthoe maa kmai"


def test_words_split_where_the_script_changes() -> None:
    assert tokens.words("君のloveだよ") == ["君の", "love", "だよ"]
    assert tokens.words("사랑hello") == ["사랑", "hello"]
    assert tokens.words("café naïve") == ["café", "naïve"]
    assert tokens.similarity_key("君はmy loveだよ", "en").split()[1:3] == ["my", "love"]


def test_tokenized_lines_carry_entry_and_letters() -> None:
    lines = lines_of(["[Chorus]", "Тело гниёт", "Просто забудь (x2)"], "ru")
    assert [(line.ordinal, line.entry) for line in lines] == [(0, 1), (1, 2), (2, 3)]
    assert lines[0].letters == 9
    assert lines[1].tokens == ("Просто", "забудь")


def test_similarity_key_romanises_cjk_only() -> None:
    assert tokens.similarity_key("Привет, Мир!", "ru") == "привет мир"
    assert tokens.similarity_key("你好世界", "zh") == "ni hao shi jie"
