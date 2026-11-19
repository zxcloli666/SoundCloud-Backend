from __future__ import annotations

import asyncio
import logging

from worker.domain.deadline import Clock

log = logging.getLogger("worker.bus.turn")


class EngineTurn:
    def __init__(self, clock: Clock, hold_s: float) -> None:
        self.owner: str | None = None
        self._clock = clock
        self._hold_s = hold_s
        self._since = clock.now()
        self._waiting: list[str] = []
        self._changed = asyncio.Event()

    def claim(self, lane: str) -> bool:
        if self.owner is None:
            self._hand(lane)
        if self.owner == lane:
            return not self._overdue()
        if lane not in self._waiting:
            self._waiting.append(lane)
        return False

    def release(self, lane: str) -> None:
        if self.owner == lane:
            self._hand(self._waiting[0] if self._waiting else None)

    async def changed(self) -> None:
        await self._changed.wait()

    def _overdue(self) -> bool:
        return bool(self._waiting) and self._clock.now() - self._since >= self._hold_s

    def _hand(self, lane: str | None) -> None:
        if lane is not None:
            log.debug("engine_turn", extra={"from_lane": self.owner, "to_lane": lane})
        if lane in self._waiting:
            self._waiting.remove(lane)
        self.owner = lane
        self._since = self._clock.now()
        self._changed.set()
        self._changed = asyncio.Event()
