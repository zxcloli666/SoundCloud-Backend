from __future__ import annotations

import pytest
from eval.lyricsfile import RefWord

from eval import words


def ref(texts: list[str], step_ms: int = 500, base_ms: int = 10000) -> list[RefWord]:
    return [
        RefWord(0, text, base_ms + index * step_ms, base_ms + (index + 1) * step_ms)
        for index, text in enumerate(texts)
    ]


def produced(
    texts: list[str],
    shift_ms: int = 0,
    step_ms: int = 500,
    base_ms: int = 10000,
    **flags: set[int],
) -> list[dict[str, object]]:
    return [
        {
            "line": 0,
            "text": text,
            "start_ms": base_ms + index * step_ms + shift_ms,
            "end_ms": base_ms + (index + 1) * step_ms + shift_ms,
            "interpolated": index in flags.get("interpolated", set()),
            "unplaced": index in flags.get("unplaced", set()),
        }
        for index, text in enumerate(texts)
    ]


TEXT = ["Paper ", "lanterns ", "drift ", "above ", "the ", "quiet ", "harbor"]


def test_exact_match_pairs_every_word() -> None:
    result = words.score(ref(TEXT), produced(TEXT), False, False)
    assert len(result.pairs) == 7
    assert result.ref_words == 7
    assert result.deltas_ms(fitted=False, interpolated=None) == [0.0] * 7


def test_normalization_ignores_case_and_punctuation() -> None:
    result = words.score(ref(["Hello, ", "World!"]), produced(["hello", "world"]), False, False)
    assert len(result.pairs) == 2


def test_missing_produced_word_leaves_one_unpaired() -> None:
    ours = produced(TEXT)
    del ours[3]
    result = words.score(ref(TEXT), ours, False, False)
    assert len(result.pairs) == 6
    assert result.ref_words == 7
    assert result.produced_words == 6


def test_extra_produced_word_is_not_paired() -> None:
    ours = produced([*TEXT[:3], "extra ", *TEXT[3:]])
    result = words.score(ref(TEXT), ours, False, False)
    assert len(result.pairs) == 7
    assert result.produced_words == 8
    assert {pair.reference_s for pair in result.pairs} == {10 + i * 0.5 for i in range(7)}


def test_hyphen_split_reference_matches_joined_produced_word() -> None:
    reference = ref(["la- ", "la- ", "'la, ", "end"])
    ours = [
        {
            "line": 0,
            "text": "la-la-la",
            "start_ms": 10000,
            "end_ms": 11500,
            "interpolated": False,
            "unplaced": False,
        },
        {
            "line": 0,
            "text": "end",
            "start_ms": 11500,
            "end_ms": 12000,
            "interpolated": False,
            "unplaced": False,
        },
    ]
    result = words.score(reference, ours, False, False)
    assert result.ref_words == 4
    assert result.produced_words == 4
    assert len(result.pairs) == 4
    assert [round(pair.produced_s, 3) for pair in result.pairs[:3]] == [10.0, 10.5, 11.0]
    assert result.deltas_ms(fitted=False, interpolated=None) == [0.0] * 4


def test_hyphen_split_produced_matches_joined_reference_word() -> None:
    reference = [RefWord(0, "well-known ", 10000, 11000), RefWord(0, "end", 11000, 11500)]
    ours = produced(["well-", "known ", "end"], step_ms=500)
    result = words.score(reference, ours, False, False)
    assert len(result.pairs) == 3


def test_per_character_matching_for_ja() -> None:
    reference = [RefWord(0, "ab", 1000, 2000), RefWord(0, "cd", 2000, 3000)]
    ours = [
        {
            "line": 0,
            "text": "abc",
            "start_ms": 1000,
            "end_ms": 2500,
            "interpolated": False,
            "unplaced": False,
        },
        {
            "line": 0,
            "text": "d",
            "start_ms": 2500,
            "end_ms": 3000,
            "interpolated": False,
            "unplaced": False,
        },
    ]
    assert len(words.score(reference, ours, False, False).pairs) == 0
    assert len(words.score(reference, ours, False, True).pairs) == 4


def test_unplaced_words_are_counted_but_not_timed() -> None:
    result = words.score(ref(TEXT), produced(TEXT, unplaced={1, 2}), False, False)
    assert result.unplaced == 2
    assert len(result.pairs) == 5
    assert words.metrics([result])["coverage"] == round(5 / 7, 4)


def test_timing_math() -> None:
    timing = words.timing([-40.0, 60.0, 150.0, 250.0, 400.0])
    assert timing["n"] == 5
    assert timing["median_abs_delta_ms"] == 150.0
    assert timing["bias_ms"] == 150.0
    assert timing["acc@50ms"] == 0.2
    assert timing["acc@100ms"] == 0.4
    assert timing["acc@200ms"] == 0.6
    assert timing["acc@300ms"] == 0.8


def test_timing_of_nothing_is_empty() -> None:
    timing = words.timing([])
    assert timing["n"] == 0
    assert timing["median_abs_delta_ms"] is None


def test_bias_is_signed() -> None:
    assert words.timing([-100.0, -200.0, -300.0])["bias_ms"] == -200.0


def test_aligned_and_interpolated_are_split() -> None:
    result = words.score(ref(TEXT), produced(TEXT, shift_ms=100, interpolated={5, 6}), False, False)
    summary = words.metrics([result])
    assert summary["aligned"]["raw"]["n"] == 5  # type: ignore[index]
    assert summary["interpolated"]["raw"]["n"] == 2  # type: ignore[index]
    assert summary["all"]["raw"]["n"] == 7  # type: ignore[index]


def test_fit_removes_a_constant_offset() -> None:
    names = [f"w{index} " for index in range(40)]
    result = words.score(ref(names), produced(names, shift_ms=2000), False, False)
    raw = words.metrics([result])["all"]["raw"]  # type: ignore[index]
    fitted = words.metrics([result])["all"]["fitted"]  # type: ignore[index]
    assert result.fit.applied
    assert raw["median_abs_delta_ms"] == 2000.0
    assert raw["bias_ms"] == 2000.0
    assert fitted["median_abs_delta_ms"] == pytest.approx(0.0, abs=5.0)
    assert fitted["acc@50ms"] == 1.0


def test_small_offset_is_not_fitted() -> None:
    names = [f"w{index} " for index in range(40)]
    result = words.score(ref(names), produced(names, shift_ms=100), False, False)
    assert not result.fit.applied
    assert words.metrics([result])["all"]["fitted"] == words.metrics([result])["all"]["raw"]


def test_summary_pools_tracks_and_counts_withheld() -> None:
    first = words.score(ref(TEXT), produced(TEXT), True, False)
    second = words.score(ref(TEXT), produced(TEXT[:5]), False, False)
    summary = words.metrics([first, second])
    assert summary["tracks"] == 2
    assert summary["withheld_by_shipped_gate"] == 1
    assert summary["ref_words"] == 14
    assert summary["paired"] == 12
    assert summary["coverage"] == round(12 / 14, 4)


def test_json_round_trip() -> None:
    result = words.score(ref(TEXT), produced(TEXT, interpolated={2}), True, False)
    assert words.from_json(words.to_json(result)) == result
    assert words.from_json(None) is None
