from __future__ import annotations

import argparse
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from worker.observability import health_file

MAX_AGE_S = 30.0
DEGRADED_GRACE_S = 600.0
LANE_DEGRADED = "degraded"


@dataclass(frozen=True)
class Verdict:
    healthy: bool
    detail: str


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="worker health")
    parser.add_argument("--path", type=Path, default=health_file.HEALTH_PATH)
    parser.add_argument("--max-age-s", type=float, default=MAX_AGE_S)
    parser.add_argument("--degraded-grace-s", type=float, default=DEGRADED_GRACE_S)
    parsed = parser.parse_args(argv)
    verdict = check(parsed.path, time.time(), parsed.max_age_s, parsed.degraded_grace_s)
    print(verdict.detail)
    return 0 if verdict.healthy else 1


def check(
    path: Path,
    now: float,
    max_age_s: float = MAX_AGE_S,
    degraded_grace_s: float = DEGRADED_GRACE_S,
) -> Verdict:
    try:
        payload = health_file.read(path)
    except FileNotFoundError:
        return Verdict(False, f"{path} missing")
    except (OSError, ValueError) as error:
        return Verdict(False, f"{path} unreadable: {error}")
    written_at = payload.get(health_file.WRITTEN_AT)
    if not isinstance(written_at, int | float):
        return Verdict(False, f"{path} has no {health_file.WRITTEN_AT}")
    age = now - written_at
    if age > max_age_s:
        return Verdict(False, f"health file is {age:.0f}s old (limit {max_age_s:.0f}s)")
    lanes = payload.get(health_file.LANES)
    if isinstance(lanes, Mapping) and lanes:
        degraded_for = [lane_degraded_for(entry, now) for entry in lanes.values()]
        if all(seconds is not None and seconds > degraded_grace_s for seconds in degraded_for):
            longest = max(seconds for seconds in degraded_for if seconds is not None)
            return Verdict(False, f"all {len(lanes)} lanes degraded for {longest:.0f}s")
    return Verdict(True, f"ok, written {age:.1f}s ago")


def lane_degraded_for(entry: object, now: float) -> float | None:
    if not isinstance(entry, Mapping) or entry.get(health_file.STATE) != LANE_DEGRADED:
        return None
    since = entry.get(health_file.SINCE)
    if not isinstance(since, int | float):
        return None
    return now - since
