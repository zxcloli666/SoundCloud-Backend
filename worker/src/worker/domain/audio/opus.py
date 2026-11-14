from __future__ import annotations

from worker.domain.audio.decode import run_tool, tail
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, TransientFailure
from worker.domain.ports import Float32Array

SAMPLE_RATE = 24_000
BITRATE = "48k"
ENCODE_S = 3.0
OGG_MAGIC = b"OggS"
ARGV = (
    "ffmpeg",
    "-v",
    "error",
    "-f",
    "f32le",
    "-ar",
    str(SAMPLE_RATE),
    "-ac",
    "1",
    "-i",
    "pipe:0",
    "-c:a",
    "libopus",
    "-b:a",
    BITRATE,
    "-vbr",
    "on",
    "-application",
    "audio",
    "-f",
    "ogg",
    "pipe:1",
)


class OpusEncoder:
    async def encode(self, samples: Float32Array, deadline: Deadline) -> bytes:
        bounded = Deadline(min(deadline.at, deadline.now() + ENCODE_S), deadline.now)
        feed = samples.astype("<f4").tobytes()
        try:
            returncode, ogg, stderr = await run_tool(list(ARGV), bounded, "encode", feed)
        except OSError as error:
            raise broken(f"ffmpeg did not start: {error}") from error
        if returncode != 0:
            raise broken(f"ffmpeg rc={returncode} {tail(stderr)}")
        if not ogg.startswith(OGG_MAGIC):
            raise broken(f"ffmpeg wrote {len(ogg)} bytes and no Ogg page")
        return ogg


def broken(detail: str) -> TransientFailure:
    return TransientFailure(Reason.INTERNAL_ERROR, detail)
