from __future__ import annotations

from worker.contract import LaneSpec

SMOOTHING = 0.3
HEADROOM = 2.0
EXPIRED_WEIGHT = 1.5
WINDOW_SHARE = 0.5


class LanePace:
    def __init__(self, lane: LaneSpec) -> None:
        self._floor_s = lane.deadline_s
        self._cap_s = max(lane.deadline_s, WINDOW_SHARE * (lane.attempt_window_s or 0.0))
        self._seconds = 0.0

    def deadline_s(self) -> float:
        return min(self._cap_s, max(self._floor_s, HEADROOM * self._seconds))

    def finished(self, seconds: float) -> None:
        self._learn(seconds)

    def expired(self, seconds: float) -> None:
        self._learn(EXPIRED_WEIGHT * seconds)

    def _learn(self, seconds: float) -> None:
        if self._seconds == 0.0:
            self._seconds = seconds
            return
        self._seconds += SMOOTHING * (seconds - self._seconds)
