from __future__ import annotations

import hashlib
from collections.abc import Sequence
from pathlib import Path

from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason

GONE = (Reason.AUDIO_NOT_FOUND, Reason.AUDIO_FORBIDDEN)


class InputsGone(PermanentFailure):
    def __init__(self, detail: str) -> None:
        super().__init__(Reason.INPUTS_GONE, detail)


async def fetch(source: AudioSource, url: str, into: Path, name: str, deadline: Deadline) -> Path:
    try:
        await source.fetch(url, into, deadline)
    except PermanentFailure as failure:
        if failure.reason in GONE:
            raise InputsGone(f"{name} {failure.detail}") from failure
        raise
    return into


def delivered(files: Sequence[tuple[str, bytes]], part_bytes: int) -> Outcome:
    blob = b"".join(data for _, data in files)
    slices = []
    offset = 0
    for name, data in files:
        slices.append({"name": name, "offset": offset, "bytes": len(data)})
        offset += len(data)
    view = memoryview(blob)
    parts = [view[start : start + part_bytes] for start in range(0, len(view), part_bytes)]
    return Outcome.ok_in_parts(
        parts,
        clip={"bytes": len(blob), "sha256": hashlib.sha256(blob).hexdigest(), "parts": len(parts)},
        files=slices,
    )
