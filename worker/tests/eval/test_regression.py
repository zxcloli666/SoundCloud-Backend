from __future__ import annotations

import json
import os
from pathlib import Path

import pytest
from eval.manifest import check, load

from eval import stats

pytestmark = pytest.mark.eval

WORKER_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = WORKER_ROOT / "eval" / "manifest.json"
BASELINE = WORKER_ROOT / "eval" / "baseline.json"
RESULTS = WORKER_ROOT / "eval" / "results"
MAX_ACC_DROP = 0.02
MAX_FALSE_ACCEPT_UPPER = 0.02
MAX_GPU_GROWTH = 0.25


@pytest.fixture(scope="module")
def baseline() -> dict[str, object]:
    if not BASELINE.is_file():
        pytest.skip("no eval/baseline.json yet")
    return json.loads(BASELINE.read_text(encoding="utf-8"))


@pytest.fixture(scope="module")
def latest() -> dict[str, object]:
    name = os.environ.get("EVAL_RESULT")
    path = Path(name) if name else newest_result()
    if path is None or not path.is_file():
        pytest.skip("no eval result to compare (EVAL_RESULT or eval/results/*.json)")
    return json.loads(path.read_text(encoding="utf-8"))


def newest_result() -> Path | None:
    if not RESULTS.is_dir():
        return None
    candidates = sorted(RESULTS.glob("*.anchored.json"), key=lambda p: p.stat().st_mtime)
    return candidates[-1] if candidates else None


def test_manifest_is_consistent() -> None:
    manifest = load(MANIFEST)
    assert check(manifest) == []
    assert manifest.by_split("calib") and manifest.by_split("control")
    data = Path(os.environ["EVAL_DATA_DIR"])
    for track in manifest.tracks:
        assert (data / track.audio).is_file(), track.audio
        assert (data / track.input_text).is_file(), track.input_text
        if track.reference_lrc:
            assert (data / track.reference_lrc).is_file(), track.reference_lrc


def test_control_accuracy_does_not_regress(
    baseline: dict[str, object], latest: dict[str, object]
) -> None:
    base = control_metrics(baseline)
    now = control_metrics(latest)
    assert now["acc@0.5"] >= base["acc@0.5"] - MAX_ACC_DROP, (now, base)


def test_false_accept_bound_stays_under_two_percent(latest: dict[str, object]) -> None:
    summary = latest["summary"]
    assert summary["n_eff_negatives"] >= stats.n_min(MAX_FALSE_ACCEPT_UPPER)
    assert summary["false_accept_wilson_upper"] <= MAX_FALSE_ACCEPT_UPPER, summary


def test_audit_track_is_accepted_with_all_lines(
    baseline: dict[str, object], latest: dict[str, object]
) -> None:
    assert latest["audit_track"] == baseline["audit_track"], "the audit track changed"
    audit = next(
        (track for track in latest["tracks"] if track["id"] == latest["audit_track"]), None
    )
    assert audit is not None, "audit track missing from the run"
    assert audit["status"] == "ok", audit
    assert audit["lines_unplaced"] == 0, audit
    assert audit["lines_total"] >= baseline["summary"]["audit"]["lines_placed"], audit


def test_gpu_cost_does_not_grow_beyond_a_quarter(
    baseline: dict[str, object], latest: dict[str, object]
) -> None:
    base_cost = sum(baseline["summary"]["gpu_s_per_audio_minute"].values())
    now_cost = sum(latest["summary"]["gpu_s_per_audio_minute"].values())
    assert now_cost <= base_cost * (1.0 + MAX_GPU_GROWTH), (now_cost, base_cost)


def control_metrics(report: dict[str, object]) -> dict[str, float]:
    summary = report["summary"]
    return dict(summary["by_split"]["control"]["line_metrics"])
