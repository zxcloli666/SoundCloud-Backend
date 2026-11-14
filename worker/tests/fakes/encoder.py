from __future__ import annotations

from worker.domain.deadline import Deadline
from worker.domain.ports import Float32Array

FAKE_OGG = b"OggS" + bytes(range(60))


class FakeEncoder:
    def __init__(self) -> None:
        self.clip = FAKE_OGG
        self.failure: Exception | None = None
        self.calls: list[tuple[Float32Array, float]] = []

    async def encode(self, samples: Float32Array, deadline: Deadline) -> bytes:
        self.calls.append((samples, deadline.remaining()))
        if self.failure is not None:
            raise self.failure
        return self.clip
