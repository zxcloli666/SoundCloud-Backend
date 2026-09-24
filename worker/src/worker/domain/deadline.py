from __future__ import annotations

import asyncio
import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from worker.domain.outcome import Reason, TransientFailure


class Clock(Protocol):
    def now(self) -> float: ...

    async def sleep(self, seconds: float) -> None: ...


class MonotonicClock:
    def now(self) -> float:
        return time.monotonic()

    async def sleep(self, seconds: float) -> None:
        await asyncio.sleep(seconds)


@dataclass(frozen=True)
class Deadline:
    at: float
    now: Callable[[], float] = time.monotonic

    @classmethod
    def after(cls, seconds: float, now: Callable[[], float] = time.monotonic) -> Deadline:
        return cls(now() + seconds, now)

    @classmethod
    def from_epoch_ms(
        cls,
        epoch_ms: int,
        wall: Callable[[], float] = time.time,
        now: Callable[[], float] = time.monotonic,
    ) -> Deadline:
        return cls(now() + (epoch_ms / 1000.0 - wall()), now)

    def remaining(self) -> float:
        return max(0.0, self.at - self.now())

    def expired(self) -> bool:
        return self.at <= self.now()

    def budget(self, cap: float) -> float:
        return min(cap, self.remaining())

    def minus(self, seconds: float) -> Deadline:
        return Deadline(self.at - seconds, self.now)

    def check(self, stage: str) -> None:
        if self.expired():
            raise TransientFailure(Reason.DEADLINE_EXCEEDED, f"stage={stage}")
