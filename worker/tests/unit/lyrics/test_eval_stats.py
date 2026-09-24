from __future__ import annotations

import pytest

from eval import stats


def test_n_min_for_two_percent_is_189() -> None:
    assert stats.n_min(0.02) == 189
    assert stats.n_min(0.05) == 73


def test_wilson_bound_with_zero_failures_needs_enough_trials() -> None:
    assert stats.wilson_upper(0, 5) > 0.4
    assert stats.wilson_upper(0, 188) > 0.02
    assert stats.wilson_upper(0, 189) <= 0.02
    assert stats.wilson_upper(0, 0) == 1.0
    assert stats.wilson_upper(10, 10) == pytest.approx(1.0)


def test_effective_sample_size_uses_the_design_effect() -> None:
    assert stats.design_effect(11, 0.1) == pytest.approx(2.0)
    assert stats.n_eff(670, 11, 0.1) == pytest.approx(335.0)
    assert stats.n_eff(100, 1, 0.1) == pytest.approx(100.0)


def test_accuracy_and_median() -> None:
    deltas = [0.1, -0.4, 0.6, 1.2]
    assert stats.accuracy_at(deltas, 0.5) == 0.5
    assert stats.accuracy_at(deltas, 1.0) == 0.75
    assert stats.accuracy_at([], 0.5) == 0.0
    assert stats.median([3.0, 1.0, 2.0]) == 2.0
    assert stats.median([3.0, 1.0, 2.0, 4.0]) == 2.5


def test_icc_is_high_for_homogeneous_clusters_and_falls_back_when_degenerate() -> None:
    homogeneous = [[1, 1, 1, 1], [0, 0, 0, 0], [1, 1, 1, 1], [0, 0, 0, 0]]
    mixed = [[1, 0, 1, 0], [0, 1, 0, 1], [1, 0, 1, 0], [0, 1, 0, 1]]
    assert stats.icc(homogeneous) > 0.9
    assert stats.icc(mixed) == 0.0
    assert stats.icc([[1]]) == stats.DEFAULT_RHO
