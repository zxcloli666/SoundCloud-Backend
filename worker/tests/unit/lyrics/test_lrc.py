from __future__ import annotations

from tests.unit.lyrics.conftest import lines_of, timing
from worker.domain.lyrics import lrc, text, tokens
from worker.domain.lyrics.placement import LineTiming

PAUSE_S = 2.5


def forty_lines() -> list[str]:
    return [f"строка номер {index} слова тут" for index in range(40)]


def placed(count: int, *, skip: set[int] = frozenset(), step: float = 4.0) -> dict[int, LineTiming]:
    return {
        line: timing(line, 5.0 + line * step, 5.0 + line * step + 2.0)
        for line in range(count)
        if line not in skip
    }


def lrc_lines(rendered: lrc.Rendered) -> list[str]:
    return rendered.synced_lrc.split("\n")


def test_forty_lines_with_one_unplaced_keep_every_line() -> None:
    script = text.parse("\n".join(forty_lines()))
    lines = lines_of(forty_lines())
    placement = placed(40, skip={17})
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    body = [line for line in lrc_lines(rendered) if not line.endswith("]")]
    assert len(body) == 40
    assert body[17] == "[01:11.00]строка номер 17 слова тут"
    assert body[16].startswith("[01:09.00]")
    unplaced = [word for word in rendered.words if word["unplaced"]]
    assert {word["line"] for word in unplaced} == {17}
    assert all(word["start_ms"] == word["end_ms"] == 71_000 for word in unplaced)


def test_leading_unplaced_lines_are_backdated_monotonically() -> None:
    script = text.parse("\n".join(forty_lines()))
    lines = lines_of(forty_lines())
    rendered = lrc.render(script, lines, placed(40, skip={0, 1}), pause_s=PAUSE_S)
    body = [line for line in lrc_lines(rendered) if not line.endswith("]")]
    assert len(body) == 40
    assert body[0].startswith("[00:12.98]")
    assert body[1].startswith("[00:12.99]")
    assert body[2].startswith("[00:13.00]")
    stamps = [line[:10] for line in lrc_lines(rendered)]
    assert stamps == sorted(stamps)


def test_markers_become_breaks_and_pauses_get_blank_lines() -> None:
    raw = "[Intro]\nПервая строка\n[Chorus]\nВторая строка\nТретья строка\nЧетвёртая строка"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"))
    placement = {
        0: timing(0, 10.0, 12.0),
        1: timing(1, 12.4, 14.0),
        2: timing(2, 30.0, 32.0),
        3: timing(3, 33.0, 35.0),
    }
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    assert lrc_lines(rendered) == [
        "[00:09.99]",
        "[00:10.00]Первая строка",
        "[00:12.00]",
        "[00:12.40]Вторая строка",
        "[00:14.00]",
        "[00:30.00]Третья строка",
        "[00:33.00]Четвёртая строка",
    ]


def test_interpolated_words_are_flagged() -> None:
    raw = "Первая строка\nВторая строка\nТретья строка"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"))
    placement = {
        0: timing(0, 1.0, 2.0),
        1: timing(1, 2.5, 3.5, source="interpolated", region=None, score=0.0, similarity=0.0),
        2: timing(2, 4.0, 5.0),
    }
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    flags = {word["line"]: word["interpolated"] for word in rendered.words}
    assert flags == {0: False, 1: True, 2: False}
    assert all(not word["unplaced"] for word in rendered.words)


def test_word_times_never_precede_the_clamped_line_start() -> None:
    raw = "Первая\nВторая"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"))
    placement = {0: timing(0, 10.0, 12.0), 1: timing(1, 9.0, 11.0)}
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    assert lrc_lines(rendered) == ["[00:10.00]Первая", "[00:10.00]Вторая"]
    assert all(word["start_ms"] >= 10_000 for word in rendered.words if word["line"] == 1)


def test_sung_lines_in_brackets_keep_their_text_and_words() -> None:
    raw = "(Oh baby)\nI want you back\n（ラララ）\n(Chorus)"
    script = text.parse(raw)
    lines = tokens.tokenize_script(script, ["en", "en", "ja", None])
    placement = {line: timing(line, 5.0 + line * 4.0, 7.0 + line * 4.0) for line in range(3)}
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    assert lrc_lines(rendered) == [
        "[00:05.00](Oh baby)",
        "[00:09.00]I want you back",
        "[00:13.00](ラララ)",
        "[00:15.00]",
    ]
    assert {word["line"] for word in rendered.words} == {0, 1, 2}


def test_timestamp_format() -> None:
    assert lrc.timestamp(0.0) == "[00:00.00]"
    assert lrc.timestamp(61.254) == "[01:01.25]"
    assert lrc.timestamp(3599.996) == "[60:00.00]"
