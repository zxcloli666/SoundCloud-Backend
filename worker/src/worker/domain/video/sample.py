from __future__ import annotations

from pathlib import Path

import numpy as np

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason
from worker.domain.ports import Uint8Array

MS = 1000.0
MIN_SPAN_S = 1.0


def window(duration_s: float, offset_ms: int | None, track_ms: int | None) -> tuple[float, float]:
    start = (offset_ms or 0) / MS
    if start > duration_s - MIN_SPAN_S:
        start = 0.0
    span = duration_s - start
    if track_ms:
        span = min(span, track_ms / MS)
    return start, min(max(span, MIN_SPAN_S), duration_s - start)


async def frames(
    path: Path,
    *,
    start_s: float,
    span_s: float,
    count: int,
    side: int,
    deadline: Deadline,
) -> Uint8Array:
    rate = count / span_s
    shape = (
        f"scale={side}:{side}:force_original_aspect_ratio=decrease,"
        f"pad={side}:{side}:(ow-iw)/2:(oh-ih)/2"
    )
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-ss", f"{start_s:.3f}", "-t", f"{span_s:.3f}",
        "-i", str(path), "-an", "-vf", f"fps={rate:.6f},{shape}", "-frames:v", str(count),
        "-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1",
    ]  # fmt: skip
    returncode, stdout, stderr = await decode.run_tool(argv, deadline, "vlm-frames")
    if returncode != 0:
        raise decode.tool_failure("ffmpeg", returncode, stderr)
    size = side * side * 3
    got = len(stdout) // size
    if got < 1:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "video gave no frames")
    return np.frombuffer(stdout, dtype=np.uint8, count=got * size).reshape(got, side, side, 3)
