from __future__ import annotations

import pytest
from eval.lyricsfile import LyricsfileError

from eval import lyricsfile

SAMPLE = """\
version: '1.0'
metadata:
  title: Harbor Song
  artist: Made Up
  album: Harbor Song
  duration_ms: 294000
  instrumental: false
  language: en
lines:
- text: Paper lanterns drift above the quiet harbor
  start_ms: 29200
  end_ms: 32964
  words:
  - text: 'Paper '
    start_ms: 29200
  - text: 'lanterns '
    start_ms: 29232
  - text: 'drift '
    start_ms: 29257
  - text: 'above '
    start_ms: 29647
  - text: 'the '
    start_ms: 29854
  - text: 'quiet '
    start_ms: 29935
  - text: harbor
    start_ms: 30200
- text: Hush now, la la
  start_ms: 33000
  end_ms: 35000
- text: la-la-la
  start_ms: 35100
  end_ms: 36500
  words:
  - text: 'la-'
    start_ms: 35100
  - text: 'la-'
    start_ms: 35500
  - text: la
    start_ms: 36000
plain: |-
  Paper lanterns drift above the quiet harbor
  Hush now, la la
  la-la-la
"""


def test_parse_reads_lines_and_words() -> None:
    parsed = lyricsfile.parse(SAMPLE)
    assert parsed.title == "Harbor Song"
    assert parsed.language == "en"
    assert parsed.duration_ms == 294000
    assert [line.start_ms for line in parsed.lines] == [29200, 33000, 35100]
    assert parsed.dropped_word_lines == 0
    assert parsed.plain.splitlines()[0] == "Paper lanterns drift above the quiet harbor"


def test_words_keep_trailing_space_and_get_ends() -> None:
    first = lyricsfile.parse(SAMPLE).lines[0]
    assert len(first.words) == 7
    assert first.words[0].text == "Paper "
    assert first.words[0].end_ms == 29232
    assert first.words[-1].text == "harbor"
    assert first.words[-1].end_ms == 32964


def test_line_without_words_is_kept_with_no_words() -> None:
    parsed = lyricsfile.parse(SAMPLE)
    assert parsed.lines[1].words == ()
    assert len(parsed.words) == 10


def test_hyphen_split_words_stay_separate() -> None:
    texts = [word.text for word in lyricsfile.parse(SAMPLE).lines[2].words]
    assert texts == ["la-", "la-", "la"]


def test_lrc_uses_lyricsfile_line_starts() -> None:
    lrc = lyricsfile.to_lrc(lyricsfile.parse(SAMPLE)).splitlines()
    assert lrc[0] == "[00:29.200]Paper lanterns drift above the quiet harbor"
    assert lrc[2] == "[00:35.100]la-la-la"


def test_words_json_round_trip() -> None:
    words = lyricsfile.parse(SAMPLE).words
    assert lyricsfile.words_from_json(lyricsfile.words_to_json(words)) == words


def test_validate_accepts_good_line() -> None:
    words = [("one ", 1000), ("two", 1500)]
    assert lyricsfile.validate("one two", 1000, 2000, words) is None


@pytest.mark.parametrize(
    ("words", "start", "end", "problem"),
    [
        ([("one ", 1500), ("two", 1500)], 1000, 2000, "do not increase"),
        ([("one ", 900), ("two", 1500)], 1000, 2000, "before its line"),
        ([("one ", 1000), ("two", 2300)], 1000, 2000, "after its line"),
        ([("one ", 1000), ("three", 1500)], 1000, 2000, "differ"),
    ],
)
def test_validate_rejects(words: list[tuple[str, int]], start: int, end: int, problem: str) -> None:
    result = lyricsfile.validate("one two", start, end, words)
    assert result is not None
    assert problem in result


def test_validate_allows_slack_after_line_end() -> None:
    assert lyricsfile.validate("one two", 1000, 2000, [("one ", 1000), ("two", 2200)]) is None


def test_invalid_words_are_dropped_but_line_is_kept() -> None:
    broken = SAMPLE.replace("start_ms: 29232", "start_ms: 29100")
    parsed = lyricsfile.parse(broken)
    assert parsed.lines[0].words == ()
    assert parsed.dropped_word_lines == 1
    assert len(parsed.lines) == 3


def test_parse_rejects_non_lyricsfile() -> None:
    with pytest.raises(LyricsfileError):
        lyricsfile.parse("just: text")
    with pytest.raises(LyricsfileError):
        lyricsfile.parse("lines: [1")
