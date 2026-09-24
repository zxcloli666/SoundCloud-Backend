from __future__ import annotations

import asyncio
import json
from pathlib import Path

from worker import health
from worker.observability.health_file import HealthFile


def test_write_is_atomic_and_stamps_written_at(tmp_path: Path) -> None:
    path = tmp_path / "run" / "health.json"
    HealthFile(path, wall=lambda: 100.0).write({"lanes": {}})
    assert json.loads(path.read_text()) == {"written_at": 100.0, "lanes": {}}
    assert not path.with_name("health.json.tmp").exists()
    verdict = health.check(path, now=120.0)
    assert verdict.healthy and "20.0s ago" in verdict.detail


def test_stale_file_is_unhealthy(tmp_path: Path) -> None:
    path = tmp_path / "health.json"
    HealthFile(path, wall=lambda: 100.0).write({})
    verdict = health.check(path, now=131.0)
    assert not verdict.healthy and "31s old" in verdict.detail


def test_missing_or_garbage_file_is_unhealthy(tmp_path: Path) -> None:
    assert not health.check(tmp_path / "none.json", now=0.0).healthy
    garbage = tmp_path / "garbage.json"
    garbage.write_text("[1, 2]")
    assert "unreadable" in health.check(garbage, now=0.0).detail
    no_stamp = tmp_path / "nostamp.json"
    no_stamp.write_text("{}")
    assert "written_at" in health.check(no_stamp, now=0.0).detail


def test_all_lanes_degraded_past_grace_is_unhealthy(tmp_path: Path) -> None:
    path = tmp_path / "health.json"
    lanes = {
        "audio": {"state": "degraded", "since": 0.0},
        "lyrics": {"state": "degraded", "since": 300.0},
    }
    HealthFile(path, wall=lambda: 1000.0).write({"lanes": lanes})
    verdict = health.check(path, now=1001.0)
    assert not verdict.healthy and "all 2 lanes degraded for 1001s" in verdict.detail
    assert health.check(path, now=1001.0, degraded_grace_s=2000.0).healthy


def test_one_serving_lane_keeps_the_worker_healthy(tmp_path: Path) -> None:
    path = tmp_path / "health.json"
    lanes = {
        "audio": {"state": "degraded", "since": 0.0},
        "lyrics": {"state": "serving", "since": 0.0},
        "encode": {"state": "not_served", "since": 0.0},
    }
    HealthFile(path, wall=lambda: 5000.0).write({"lanes": lanes})
    assert health.check(path, now=5001.0).healthy


async def test_run_writes_until_stopped(tmp_path: Path) -> None:
    path = tmp_path / "health.json"
    ticks = iter(range(1, 100))
    stop = asyncio.Event()
    writer = HealthFile(path, wall=lambda: float(next(ticks)))
    task = asyncio.create_task(writer.run(lambda: {"n": 1}, stop, interval_s=0.02))
    await asyncio.sleep(0.07)
    stop.set()
    await task
    written = json.loads(path.read_text())
    assert written["n"] == 1 and written["written_at"] >= 3


def test_main_returns_exit_codes(tmp_path: Path, capsys: object) -> None:
    path = tmp_path / "health.json"
    assert health.main(["--path", str(path)]) == 1
    HealthFile(path).write({"lanes": {}})
    assert health.main(["--path", str(path)]) == 0
