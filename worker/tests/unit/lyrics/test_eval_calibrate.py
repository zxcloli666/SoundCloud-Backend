from __future__ import annotations

import json
import random
from pathlib import Path

import pytest

from eval import calibrate


def separable_rows(count: int = 400) -> list[tuple[list[float], int]]:
    rng = random.Random(7)
    rows: list[tuple[list[float], int]] = []
    for _ in range(count):
        anchor = rng.random()
        others = [rng.random() for _ in range(6)]
        features = [others[0], others[1], others[2], anchor, others[3], 1.0, others[5]]
        rows.append((features, 1 if anchor + rng.gauss(0.0, 0.1) > 0.5 else 0))
    return rows


def test_fit_recovers_the_informative_feature() -> None:
    weights = calibrate.fit(separable_rows())
    assert weights.values[3] > 2.0
    assert all(
        abs(value) < weights.values[3] / 2
        for index, value in enumerate(weights.values)
        if index != 3
    )
    assert calibrate.fit([]).as_config() == {
        name: 0.0 for name in (*calibrate.FEATURE_NAMES, "bias")
    }


def test_calibrated_confidence_is_the_mean_line_sigmoid() -> None:
    weights = calibrate.Weights((0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0), -2.0)
    features = {"0": [1, 1, 1, 1.0, 1, 1, 1], "1": [1, 1, 1, 0.0, 1, 1, 1]}
    expected = (calibrate.sigmoid(2.0) + calibrate.sigmoid(-2.0)) / 2
    assert calibrate.calibrated_confidence(features, weights) == pytest.approx(expected, abs=1e-3)
    assert calibrate.calibrated_confidence({}, weights) == 0.0


def test_gate_reopens_only_for_low_confidence_with_clean_words() -> None:
    clean = {"collapsed_share": 0.1, "rate_outliers": 0.0, "separated": True}
    assert calibrate.gate_open("ok", None, {})
    assert calibrate.gate_open("rejected", "low_confidence", clean)
    assert not calibrate.gate_open("rejected", "lyrics_mismatch", clean)
    assert not calibrate.gate_open("rejected", "low_confidence", {**clean, "collapsed_share": 0.3})
    assert not calibrate.gate_open(
        "rejected", "low_confidence", {**clean, "collapsed_share": 0.22, "separated": False}
    )


def scored(
    identifier: str, expected: str, calibrated: float, accuracy: float | None, split: str = "calib"
) -> calibrate.ScoredTrack:
    return calibrate.ScoredTrack(
        id=identifier,
        split=split,
        cluster=identifier.split(":")[0],
        expected=expected,
        status="ok" if expected == "ok" else "rejected",
        reason=None if expected == "ok" else "low_confidence",
        calibrated=calibrated,
        acc_at_half=accuracy,
        gate_open=True,
    )


def test_recommendation_maximises_acceptance_under_the_false_accept_cap() -> None:
    items = [
        scored("p1", "ok", 0.9, 0.95),
        scored("p2", "ok", 0.7, 0.9),
        scored("p3", "ok", 0.55, 0.6),
        scored("a:n1", "lyrics_mismatch", 0.6, None),
        scored("a:n2", "lyrics_mismatch", 0.4, None),
    ]
    assert calibrate.recommend(items) == 0.65
    row = calibrate.threshold_row(items, "calib", 0.65)
    assert row["accept_rate"] == pytest.approx(2 / 3, abs=1e-3)
    assert row["false_accept"] == 0.0
    assert calibrate.threshold_row(items, "control", 0.65)["accept_rate"] is None


def test_main_writes_weights_and_thresholds(tmp_path: Path) -> None:
    rows = separable_rows(120)
    tracks = [
        {
            "id": f"t{index}",
            "split": "calib" if index % 2 == 0 else "control",
            "cluster": f"t{index}",
            "expected": "ok",
            "status": "ok",
            "reason": None,
            "metrics": {"separated": True},
            "acc@0.5": 0.9,
            "line_deltas": {str(line): 0.1 if hit else 2.0 for line, (_, hit) in enumerate(rows)},
            "line_features": {str(line): features for line, (features, _) in enumerate(rows)},
        }
        for index in range(4)
    ]
    result = tmp_path / "run.json"
    result.write_text(json.dumps({"sync_version": "s2.x", "tracks": tracks}), encoding="utf-8")
    assert calibrate.main([str(result)]) == 0
    payload = json.loads((tmp_path / "run.calibration.json").read_text(encoding="utf-8"))
    assert payload["calib_lines"] == 240
    assert payload["confidence_weights"]["anchor"] > 1.0
    assert len(payload["thresholds"]["calib"]) == len(calibrate.THRESHOLDS)
    assert payload["recommended_min_confidence"] is not None
