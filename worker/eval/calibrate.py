from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from eval import stats

WORKER_ROOT = Path(__file__).resolve().parent.parent
FEATURE_NAMES = (
    "aligned_share",
    "inside_share",
    "collapsed",
    "anchor",
    "aligner",
    "separated",
    "order",
)
LINE_HIT_S = 0.5
L2 = 1.0
NEWTON_STEPS = 25
THRESHOLDS = tuple(round(0.30 + 0.05 * step, 2) for step in range(14))
MAX_POINT_FALSE_ACCEPT = 0.01
MIN_ACCEPTED_ACCURACY = 0.85
MIX_PENALTY = 0.05
MAX_COLLAPSED_SHARE = 0.25
MAX_RATE_OUTLIERS = 0.10


@dataclass(frozen=True)
class Weights:
    values: tuple[float, ...]
    bias: float

    def as_config(self) -> dict[str, float]:
        return {**dict(zip(FEATURE_NAMES, self.values, strict=True)), "bias": self.bias}


@dataclass(frozen=True)
class ScoredTrack:
    id: str
    split: str
    cluster: str
    expected: str
    status: str
    reason: str | None
    calibrated: float
    acc_at_half: float | None
    gate_open: bool


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="eval.calibrate")
    parser.add_argument("result", type=Path)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args(argv)
    report = json.loads(args.result.read_text(encoding="utf-8"))
    tracks = list(report["tracks"])
    weights = fit(line_rows(tracks, "calib"))
    scored = [score(track, weights) for track in tracks]
    payload = {
        "source": str(args.result),
        "sync_version": report.get("sync_version"),
        "calib_lines": len(line_rows(tracks, "calib")),
        "confidence_weights": weights.as_config(),
        "thresholds": {
            split: [threshold_row(scored, split, t) for t in THRESHOLDS]
            for split in ("calib", "control")
        },
        "recommended_min_confidence": recommend(scored),
        "false_accept_icc_calib": false_accept_icc(scored),
        "tracks": [track_row(item) for item in scored],
    }
    out = args.out or args.result.with_name(args.result.stem + ".calibration.json")
    out.write_text(json.dumps(payload, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print(json.dumps({key: payload[key] for key in payload if key != "tracks"}, indent=1))
    print(f"written {out}")
    return 0


def line_rows(tracks: Sequence[Mapping[str, object]], split: str) -> list[tuple[list[float], int]]:
    rows: list[tuple[list[float], int]] = []
    for track in tracks:
        if track.get("split") != split or track.get("expected") != "ok":
            continue
        deltas = dict(track.get("line_deltas") or {})
        features = dict(track.get("line_features") or {})
        for line, row in features.items():
            if line not in deltas:
                continue
            hit = 1 if abs(float(deltas[line])) <= LINE_HIT_S else 0
            rows.append(([float(value) for value in row], hit))
    return rows


def fit(rows: Sequence[tuple[Sequence[float], int]], l2: float = L2) -> Weights:
    if not rows:
        return Weights(tuple(0.0 for _ in FEATURE_NAMES), 0.0)
    features = np.array([list(row) for row, _ in rows], dtype=np.float64)
    labels = np.array([label for _, label in rows], dtype=np.float64)
    design = np.hstack([features, np.ones((features.shape[0], 1))])
    theta = np.zeros(design.shape[1])
    ridge = np.eye(design.shape[1]) * l2
    ridge[-1, -1] = 0.0
    for _ in range(NEWTON_STEPS):
        probabilities = sigmoid(design @ theta)
        gradient = design.T @ (probabilities - labels) + ridge @ theta
        curvature = probabilities * (1.0 - probabilities)
        hessian = (design * curvature[:, None]).T @ design + ridge
        step = np.linalg.solve(hessian + np.eye(design.shape[1]) * 1e-9, gradient)
        theta = theta - step
        if float(np.max(np.abs(step))) < 1e-8:
            break
    return Weights(tuple(round(float(v), 4) for v in theta[:-1]), round(float(theta[-1]), 4))


def calibrated_confidence(
    features: Mapping[str, Sequence[float]] | Mapping[int, Sequence[float]], weights: Weights
) -> float:
    if not features:
        return 0.0
    scores = [
        sigmoid(weights.bias + sum(w * float(x) for w, x in zip(weights.values, row, strict=True)))
        for row in features.values()
    ]
    return round(float(np.mean(scores)), 3)


def score(track: Mapping[str, object], weights: Weights) -> ScoredTrack:
    metrics = dict(track.get("metrics") or {})
    reason = track.get("reason")
    return ScoredTrack(
        id=str(track["id"]),
        split=str(track["split"]),
        cluster=str(track.get("cluster") or str(track["id"]).split(":")[0]),
        expected=str(track["expected"]),
        status=str(track["status"]),
        reason=str(reason) if reason else None,
        calibrated=calibrated_confidence(dict(track.get("line_features") or {}), weights),
        acc_at_half=float(track["acc@0.5"]) if track.get("acc@0.5") is not None else None,
        gate_open=gate_open(str(track["status"]), reason, metrics),
    )


def gate_open(status: str, reason: object, metrics: Mapping[str, object]) -> bool:
    if status == "ok":
        return True
    if reason != "low_confidence":
        return False
    penalty = 0.0 if metrics.get("separated", True) else MIX_PENALTY
    collapsed = float(metrics.get("collapsed_share", 0.0) or 0.0)
    outliers = float(metrics.get("rate_outliers", 0.0) or 0.0)
    return collapsed <= MAX_COLLAPSED_SHARE - penalty and outliers <= MAX_RATE_OUTLIERS - penalty


def accepted_at(item: ScoredTrack, threshold: float) -> bool:
    return item.gate_open and item.calibrated >= threshold


def threshold_row(scored: Sequence[ScoredTrack], split: str, threshold: float) -> dict[str, object]:
    subset = [item for item in scored if item.split == split]
    positives = [item for item in subset if item.expected == "ok"]
    negatives = [item for item in subset if item.expected != "ok"]
    accepted = [item for item in positives if accepted_at(item, threshold)]
    accuracies = [item.acc_at_half for item in accepted if item.acc_at_half is not None]
    return {
        "min_confidence": threshold,
        "accept_rate": share(len(accepted), len(positives)),
        "false_accept": share(
            sum(1 for item in negatives if accepted_at(item, threshold)), len(negatives)
        ),
        "acc@0.5_accepted": round(float(np.mean(accuracies)), 4) if accuracies else None,
    }


def recommend(scored: Sequence[ScoredTrack]) -> float | None:
    best: tuple[float, float] | None = None
    for threshold in THRESHOLDS:
        row = threshold_row(scored, "calib", threshold)
        accept = row["accept_rate"]
        false_accept = row["false_accept"]
        accuracy = row["acc@0.5_accepted"]
        if accept is None or accuracy is None:
            continue
        if (false_accept or 0.0) > MAX_POINT_FALSE_ACCEPT or accuracy < MIN_ACCEPTED_ACCURACY:
            continue
        if best is None or float(accept) > best[1]:
            best = (threshold, float(accept))
    return best[0] if best else None


def false_accept_icc(scored: Sequence[ScoredTrack]) -> float:
    clusters: dict[str, list[int]] = {}
    for item in scored:
        if item.split != "calib" or item.expected == "ok":
            continue
        clusters.setdefault(item.cluster, []).append(1 if item.status == "ok" else 0)
    return round(stats.icc(list(clusters.values())), 4)


def track_row(item: ScoredTrack) -> dict[str, object]:
    return {
        "id": item.id,
        "split": item.split,
        "expected": item.expected,
        "status": item.status,
        "reason": item.reason,
        "calibrated": item.calibrated,
        "gate_open": item.gate_open,
    }


def share(count: int, total: int) -> float | None:
    return round(count / total, 4) if total else None


def sigmoid(value: float | np.ndarray) -> float | np.ndarray:
    return 1.0 / (1.0 + np.exp(-value))


if __name__ == "__main__":
    sys.exit(main())
