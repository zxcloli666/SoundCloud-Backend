from __future__ import annotations

import asyncio
import heapq
from itertools import count


class FakeClock:
    def __init__(self, start: float = 1_000.0) -> None:
        self._now = start
        self._sleepers: list[tuple[float, int, asyncio.Future[None]]] = []
        self._order = count()

    def now(self) -> float:
        return self._now

    async def sleep(self, seconds: float) -> None:
        if seconds <= 0:
            await asyncio.sleep(0)
            return
        future: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        heapq.heappush(self._sleepers, (self._now + seconds, next(self._order), future))
        await future

    def advance(self, seconds: float) -> None:
        self._now += seconds
        while self._sleepers and self._sleepers[0][0] <= self._now:
            _, _, future = heapq.heappop(self._sleepers)
            if not future.done():
                future.set_result(None)

    async def tick(self, seconds: float = 0.0, turns: int = 10) -> None:
        self.advance(seconds)
        for _ in range(turns):
            await asyncio.sleep(0)

    @property
    def sleeping(self) -> int:
        return sum(1 for _, _, future in self._sleepers if not future.done())
