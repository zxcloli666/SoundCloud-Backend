from __future__ import annotations

import pytest

from eval import reference


def pairs(produced: list[float], expected: list[float]) -> dict[int, tuple[float, float]]:
    return {index: pair for index, pair in enumerate(zip(produced, expected, strict=True))}


EXPECTED = [10.0 + 4.3 * index for index in range(20)]
LINES = [reference.ReferenceLine(moment, 0.6) for moment in EXPECTED]


def test_a_reference_from_another_intro_is_shifted_back() -> None:
    produced = [moment + 18.5 for moment in EXPECTED]
    produced[3] += 6.0
    produced[11] -= 4.0
    fit = reference.fit(pairs(produced, EXPECTED))
    assert fit.applied
    assert fit.scale == 1.0
    assert fit.shift_s == pytest.approx(18.5)
    assert fit.inlier_share == pytest.approx(0.9)


def test_a_reference_at_another_speed_is_rescaled() -> None:
    produced = [moment * 0.96 + 0.4 for moment in EXPECTED]
    fit = reference.fit(pairs(produced, EXPECTED))
    assert fit.applied
    assert fit.scale == pytest.approx(0.96, abs=0.003)
    assert fit.inlier_share == 1.0


def test_a_matching_reference_is_left_alone() -> None:
    produced = [moment + 0.2 for moment in EXPECTED]
    fit = reference.fit(pairs(produced, EXPECTED))
    assert not fit.applied
    assert fit.expected(42.0) == 42.0


def test_no_correction_when_no_single_mapping_explains_most_lines() -> None:
    produced = [moment + (7.0 * (index % 3)) for index, moment in enumerate(EXPECTED)]
    assert not reference.fit(pairs(produced, EXPECTED)).applied


def test_too_few_lines_are_never_fitted() -> None:
    assert reference.fit(pairs([30.0, 34.0], [10.0, 14.0])) == reference.IDENTITY


def test_whole_second_references_are_coarse() -> None:
    assert reference.coarse([21.02, 26.05, 31.06, 35.01, 40.05, 44.05])
    assert not reference.coarse([21.42, 26.05, 31.66, 35.31, 40.85, 44.25])


def test_two_long_runs_at_different_offsets_mean_another_edit() -> None:
    produced = [moment + (0.1 if index < 10 else 16.0) for index, moment in enumerate(EXPECTED)]
    check = reference.check(pairs(produced, EXPECTED), LINES)
    assert not check.usable
    assert check.flags == ("edited",)


def test_lines_faster_than_anyone_sings_mean_a_broken_reference() -> None:
    rushed = [
        reference.ReferenceLine(moment, 5.0 if index % 4 else 0.5)
        for index, moment in enumerate(EXPECTED)
    ]
    assert reference.dense(rushed)
    assert not reference.dense(LINES)


def test_deltas_are_measured_against_the_fitted_reference() -> None:
    produced = [moment + 2.0 for moment in EXPECTED]
    check = reference.check(pairs(produced, EXPECTED), LINES)
    assert check.usable
    assert all(abs(delta) < 1e-6 for delta in check.deltas(pairs(produced, EXPECTED)).values())
