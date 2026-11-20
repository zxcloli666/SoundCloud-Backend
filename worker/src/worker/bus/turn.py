from __future__ import annotations

import asyncio
import logging
from collections.abc import Mapping

from worker.domain.deadline import Clock

log = logging.getLogger("worker.bus.turn")


class EngineTurn:
    def __init__(
        self,
        clock: Clock,
        hold_s: float,
        probe_s: float,
        lanes: Mapping[str, str],
        home: str = "",
    ) -> None:
        self.owner: str | None = None
        self.probe_s = probe_s
        self._clock = clock
        self._hold_s = hold_s
        self._lanes = lanes
        self._home = home
        self._since = clock.now()
        self._holders: set[str] = set()
        self._waiting: list[str] = []
        self._changed = asyncio.Event()

    def needs_proof(self, lane: str) -> bool:
        group = self._lanes[lane]
        if self.owner == group:
            return bool(self._waiting)
        return group != self._home

    def claim(self, lane: str, wanted: bool) -> bool:
        if not wanted:
            return False
        group = self._lanes[lane]
        if self.owner is None:
            self._hand(group)
        if self.owner == group:
            if self._overdue():
                return False
            self._holders.add(lane)
            return True
        if group not in self._waiting:
            self._waiting.append(group)
        return False

    def release(self, lane: str) -> None:
        self._holders.discard(lane)
        if self._lanes[lane] == self.owner and not self._holders and self._waiting:
            self._hand(self._waiting[0])

    async def changed(self) -> None:
        await self._changed.wait()

    def _overdue(self) -> bool:
        return bool(self._waiting) and self._clock.now() - self._since >= self._hold_s

    def _hand(self, group: str) -> None:
        log.info("engine_turn", extra={"from_group": self.owner, "to_group": group})
        if group in self._waiting:
            self._waiting.remove(group)
        self.owner = group
        self._since = self._clock.now()
        self._changed.set()
        self._changed = asyncio.Event()
