from __future__ import annotations

import re

from tests.unit.lyrics.conftest import lines_of, timing
from worker.domain.lyrics import lrc, text, tokens
from worker.domain.lyrics.placement import LineTiming, Word
from worker.domain.lyrics.tokens import TokenizedLine

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


WORD_TAG = re.compile(r"<\d\d:\d\d\.\d\d>")
TAG_TIME = re.compile(r"<(\d\d):(\d\d)\.(\d\d)>")


def worded(
    line: TokenizedLine,
    start_s: float,
    step_s: float,
    *,
    source: str = "qwen",
    end_s: float | None = None,
) -> LineTiming:
    words = tuple(
        Word(
            line.ordinal,
            token,
            round(start_s + index * step_s, 3),
            round(start_s + (index + 1) * step_s, 3),
            0.9,
        )
        for index, token in enumerate(line.tokens)
    )
    last = words[-1].end_s if words else start_s
    return LineTiming(
        line=line.ordinal,
        start_s=start_s,
        end_s=last if end_s is None else end_s,
        source=source,  # type: ignore[arg-type]
        words=words,
        region=0,
        region_score=0.9,
        anchor_similarity=0.9,
    )


def tag_seconds(tag: str) -> float:
    match = TAG_TIME.fullmatch(tag)
    assert match
    return int(match[1]) * 60 + int(match[2]) + int(match[3]) / 100


def test_every_word_of_a_placed_line_gets_a_start_tag_and_the_line_a_closing_one() -> None:
    raw = "Hello, brave world!\nla la la"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"), "en")
    placement = {0: worded(lines[0], 10.0, 0.5), 1: worded(lines[1], 12.0, 0.25)}
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    assert rendered.synced_words.split("\n") == [
        "[00:10.00]<00:10.00>Hello, <00:10.50>brave <00:11.00>world!<00:11.50>",
        "[00:12.00]<00:12.00>la <00:12.25>la <00:12.50>la<00:12.75>",
    ]


def test_stripping_the_tags_gives_exactly_the_synced_lrc() -> None:
    raw = "[Intro]\nalpha beta\n(gamma delta)\nepsilon\n[Outro]\nzeta eta theta"
    script = text.parse(raw)
    lines = tokens.tokenize_script(script, ["en"] * len(script.entries))
    placement = {
        0: worded(lines[0], 10.0, 0.4),
        1: worded(lines[1], 12.0, 0.4),
        2: timing(2, 30.0, 31.0),
        3: worded(lines[3], 40.0, 0.5),
    }
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    assert WORD_TAG.sub("", rendered.synced_words) == rendered.synced_lrc
    assert rendered.synced_words != rendered.synced_lrc


def test_lines_without_whole_real_timings_stay_untagged() -> None:
    raw = "alpha beta\ngamma delta\nepsilon zeta\neta theta\niota kappa"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"), "en")
    partial = worded(lines[1], 6.0, 0.5)
    partial = LineTiming(
        partial.line,
        partial.start_s,
        partial.end_s,
        partial.source,
        partial.words[:1],
        partial.region,
        partial.region_score,
        partial.anchor_similarity,
    )
    mismatched = worded(lines[3], 14.0, 0.5)
    mismatched = LineTiming(
        mismatched.line,
        mismatched.start_s,
        mismatched.end_s,
        mismatched.source,
        tuple(Word(3, "nowhere", w.start_s, w.end_s, w.score) for w in mismatched.words),
        mismatched.region,
        mismatched.region_score,
        mismatched.anchor_similarity,
    )
    placement = {
        0: worded(lines[0], 2.0, 0.5, source="interpolated"),
        1: partial,
        3: mismatched,
        4: worded(lines[4], 18.0, 0.5),
    }
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    tagged = [
        bool(WORD_TAG.search(line))
        for line in rendered.synced_words.split("\n")
        if not line.endswith("]")
    ]
    assert tagged == [False, False, False, False, True]
    assert WORD_TAG.sub("", rendered.synced_words) == rendered.synced_lrc


def test_word_tags_are_ordered_and_stay_inside_the_line() -> None:
    raw = "one two three\nfour five"
    script = text.parse(raw)
    lines = lines_of(raw.split("\n"), "en")
    wild = worded(lines[0], 10.0, 0.5, end_s=11.0)
    wild = LineTiming(
        wild.line,
        wild.start_s,
        wild.end_s,
        wild.source,
        (
            Word(0, "one", 9.0, 9.5, 0.9),
            Word(0, "two", 12.0, 12.5, 0.9),
            Word(0, "three", 10.5, 11.0, 0.9),
        ),
        wild.region,
        wild.region_score,
        wild.anchor_similarity,
    )
    placement = {0: wild, 1: worded(lines[1], 20.0, 1.0)}
    rendered = lrc.render(script, lines, placement, pause_s=PAUSE_S)
    for row in rendered.synced_words.split("\n"):
        stamps = re.findall(r"[\[<](\d\d:\d\d\.\d\d)[\]>]", row)
        seconds = [tag_seconds(f"<{stamp}>") for stamp in stamps]
        assert seconds == sorted(seconds)
        assert all(seconds[0] <= value <= seconds[-1] for value in seconds)
    assert rendered.synced_words.split("\n")[0] == (
        "[00:10.00]<00:10.00>one <00:11.00>two <00:11.00>three<00:11.00>"
    )
