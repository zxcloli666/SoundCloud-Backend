from __future__ import annotations

import pytest

from worker.domain.lyrics import text


def test_markers_become_breaks_and_sung_lines_keep_source_index() -> None:
    script = text.parse("[Verse 1]\nПервая строка\n\n(Chorus)\nВторая строка\r\n")
    kinds = [(line.kind, line.text, line.source) for line in script.entries]
    assert kinds == [
        ("marker", "[Verse 1]", 0),
        ("sung", "Первая строка", 1),
        ("marker", "(Chorus)", 3),
        ("sung", "Вторая строка", 4),
    ]
    assert [line.text for line in script.sung] == ["Первая строка", "Вторая строка"]
    assert script.received_lines == 4


@pytest.mark.parametrize(
    ("raw", "body", "count"),
    [
        ("Просто забудь меня (x2)", "Просто забудь меня", 2),
        ("Просто забудь меня x3", "Просто забудь меня", 3),
        ("Просто забудь меня ×2", "Просто забудь меня", 2),
        ("Просто забудь меня [2x]", "Просто забудь меня", 2),
        ("Просто забудь меня - х2", "Просто забудь меня", 2),
        ("Я люблю тебя (Х2)", "Я люблю тебя", 2),
        ("Я люблю тебя Х3", "Я люблю тебя", 3),
        ("Matrix 2", "Matrix 2", 1),
        ("Level x", "Level x", 1),
        ("x2", "", 2),
        ("Ещё раз (x99)", "Ещё раз", text.MAX_REPEATS),
    ],
)
def test_repeat_suffix_expands_the_line(raw: str, body: str, count: int) -> None:
    assert text.split_repeat(raw) == (body, count)


@pytest.mark.parametrize("raw", ["[Chorus] x2", "[Припев] x2", "[Chorus] (x2)", "(Chorus) Х2"])
def test_a_repeated_section_marker_stays_a_single_marker(raw: str) -> None:
    script = text.parse(f"{raw}\nПервая строка")
    assert [(line.kind, line.text) for line in script.entries] == [
        ("marker", raw),
        ("sung", "Первая строка"),
    ]


def test_repeated_line_appears_n_times_in_order() -> None:
    script = text.parse("A\nB (x2)\nC")
    assert [line.text for line in script.sung] == ["A", "B", "B", "C"]
    assert [line.index for line in script.entries] == [0, 1, 2, 3]
    assert script.received_lines == 3


def test_nfkc_and_whitespace_are_normalised() -> None:
    script = text.parse("Ｈｅｌｌｏ　　ｗｏｒｌｄ\n  spaced   out  ")
    assert [line.text for line in script.sung] == ["Hello world", "spaced out"]


def test_lines_total_grows_by_the_lines_cut_by_jobs() -> None:
    script = text.parse("[Intro]\nA\nB (x2)")
    assert script.lines_total(reference_lines_total=3) == 3
    assert script.lines_total(reference_lines_total=10) == 10
    assert script.lines_total(reference_lines_total=1) == 3


def test_text_without_sung_lines_is_empty() -> None:
    assert text.parse("[Instrumental]\n\n(Chorus)").sung == ()
    assert text.parse("   \n\n").entries == ()


def test_lines_without_letters_or_digits_and_bare_repeats_are_markers() -> None:
    script = text.parse("Line one\n\nx2\n* * *\n...\n♪♪♪\n—\n×3")
    assert [line.text for line in script.sung] == ["Line one"]
    assert [line.kind for line in script.entries] == ["sung"] + ["marker"] * 6
    assert script.received_lines == 7
    assert text.parse("♪♪♪\n* * *\n...").sung == ()


def test_sung_lines_in_round_brackets_stay_sung() -> None:
    script = text.parse("(Oh baby)\nI want you back\n(I want you back)\n（ラララ）\n(Chorus)")
    assert [line.text for line in script.sung] == [
        "(Oh baby)",
        "I want you back",
        "(I want you back)",
        "(ラララ)",
    ]


def test_lrc_timestamps_are_cut_from_the_reference() -> None:
    script = text.parse("[00:05.10]\n[00:12.34]Hello there\n[01:02:50] Second line")
    assert [line.text for line in script.sung] == ["Hello there", "Second line"]
    assert script.entries[0].kind == "marker"
    assert script.received_lines == 3


@pytest.mark.parametrize("separator", ["\r", "\N{LINE SEPARATOR}", "\x85", "\v"])
def test_every_unicode_line_break_starts_a_new_line(separator: str) -> None:
    script = text.parse(separator.join(["line one", "line two", "line three"]))
    assert [(line.text, line.source) for line in script.sung] == [
        ("line one", 0),
        ("line two", 1),
        ("line three", 2),
    ]
