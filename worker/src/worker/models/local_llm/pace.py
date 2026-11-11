from __future__ import annotations

import math
import time

from worker.runtime.protocol import CALL_GUARD


class Pace:
    def __init__(self, sample_tokens: int) -> None:
        self._sample_tokens = sample_tokens
        self._seconds_per_token = 0.0

    @property
    def known(self) -> bool:
        return self._seconds_per_token > 0.0

    def guard(self, tokens: int) -> None:
        CALL_GUARD.check(tokens * self._seconds_per_token)

    def fitting(self, tokens: int) -> int:
        part = int(min(tokens, self._affordable()))
        fitting = part if part >= self._sample_tokens else tokens
        self.guard(fitting)
        return fitting

    def learn(self, tokens: int, seconds: float) -> None:
        if tokens >= self._sample_tokens:
            self._seconds_per_token = seconds / tokens

    def _affordable(self) -> float:
        if not self.known:
            return math.inf
        return (CALL_GUARD.expires_at - time.monotonic()) / self._seconds_per_token
