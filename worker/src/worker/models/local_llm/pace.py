from __future__ import annotations

from worker.runtime.protocol import CALL_GUARD


class Pace:
    def __init__(self, full_tokens: int) -> None:
        self._full_tokens = full_tokens
        self._seconds_per_token = 0.0

    def guard(self, tokens: int) -> None:
        CALL_GUARD.check(tokens * self._seconds_per_token)

    def learn(self, tokens: int, seconds: float) -> None:
        if tokens >= self._full_tokens:
            self._seconds_per_token = seconds / tokens
