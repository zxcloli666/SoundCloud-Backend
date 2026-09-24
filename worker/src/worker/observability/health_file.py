from __future__ import annotations

import asyncio
import os
import time
from collections.abc import Callable, Mapping
from pathlib import Path

import orjson

HEALTH_PATH = Path("/run/worker/health.json")
WRITE_INTERVAL_S = 5.0
WRITTEN_AT = "written_at"
LANES = "lanes"
STATE = "state"
SINCE = "since"


class HealthFile:
    def __init__(self, path: Path = HEALTH_PATH, wall: Callable[[], float] = time.time) -> None:
        self._path = path
        self._wall = wall

    @property
    def path(self) -> Path:
        return self._path

    def write(self, payload: Mapping[str, object]) -> None:
        record = {WRITTEN_AT: self._wall(), **payload}
        self._path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self._path.with_name(self._path.name + ".tmp")
        temporary.write_bytes(orjson.dumps(record, default=str))
        os.replace(temporary, self._path)

    async def run(
        self,
        snapshot: Callable[[], Mapping[str, object]],
        stop: asyncio.Event,
        interval_s: float = WRITE_INTERVAL_S,
    ) -> None:
        while not stop.is_set():
            self.write(snapshot())
            waiter = asyncio.create_task(stop.wait())
            done, _ = await asyncio.wait({waiter}, timeout=interval_s)
            if not done:
                waiter.cancel()
        self.write(snapshot())


def read(path: Path) -> dict[str, object]:
    loaded = orjson.loads(path.read_bytes())
    if not isinstance(loaded, dict):
        raise ValueError(f"{path}: health payload must be an object")
    return loaded
