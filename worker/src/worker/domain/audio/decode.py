from __future__ import annotations

import asyncio
import importlib
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.domain.ports import Float32Array, Int16Array

SOXR = importlib.import_module("soxr")
MAX_CHANNELS = 2
DURATION_PROBE_MARGIN_S = 1.0
STDERR_TAIL_CHARS = 200
PCM16_SCALE = 32768.0
SILENCE_FLOOR = 1e-10


@dataclass(frozen=True)
class Pcm:
    samples: Float32Array
    sample_rate: int

    @property
    def frames(self) -> int:
        return int(self.samples.shape[0])

    @property
    def channels(self) -> int:
        return int(self.samples.shape[1])

    @property
    def duration_s(self) -> float:
        return self.frames / self.sample_rate


@dataclass(frozen=True)
class StreamInfo:
    sample_rate: int
    channels: int


async def decode(path: Path, *, max_duration_s: float, deadline: Deadline) -> Pcm:
    info = await probe(path, deadline)
    channels = min(info.channels, MAX_CHANNELS)
    argv = [
        "ffmpeg",
        "-nostdin",
        "-v",
        "error",
        "-i",
        str(path),
        "-map",
        "0:a:0",
        "-t",
        f"{max_duration_s + DURATION_PROBE_MARGIN_S:.3f}",
        "-ac",
        str(channels),
        "-ar",
        str(info.sample_rate),
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "pipe:1",
    ]
    returncode, stdout, stderr = await run_tool(argv, deadline, "decode")
    if returncode != 0:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, f"ffmpeg rc={returncode} {tail(stderr)}")
    frames = len(stdout) // (4 * channels)
    if frames == 0:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "no audio frames")
    samples = np.frombuffer(stdout, dtype=np.float32, count=frames * channels)
    if not np.all(np.isfinite(samples)):
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "non-finite samples")
    pcm = Pcm(samples.reshape(frames, channels), info.sample_rate)
    if pcm.duration_s > max_duration_s:
        raise PermanentFailure(Reason.AUDIO_TOO_LONG, f"duration_s>{max_duration_s:g}")
    return pcm


async def probe(path: Path, deadline: Deadline) -> StreamInfo:
    argv = [
        "ffprobe",
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=sample_rate,channels",
        "-of",
        "json",
        str(path),
    ]
    returncode, stdout, stderr = await run_tool(argv, deadline, "probe")
    if returncode != 0:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, f"ffprobe rc={returncode} {tail(stderr)}")
    streams = json.loads(stdout or b"{}").get("streams") or []
    if not streams:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "no audio stream")
    sample_rate = int(streams[0].get("sample_rate") or 0)
    channels = int(streams[0].get("channels") or 0)
    if sample_rate <= 0 or channels <= 0:
        raise PermanentFailure(
            Reason.UNDECODABLE_AUDIO, f"sample_rate={sample_rate} channels={channels}"
        )
    return StreamInfo(sample_rate, channels)


async def run_tool(argv: list[str], deadline: Deadline, stage: str) -> tuple[int, bytes, bytes]:
    deadline.check(stage)
    process = await asyncio.create_subprocess_exec(
        *argv,
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        stdout, stderr = await asyncio.wait_for(process.communicate(), deadline.remaining())
    except TimeoutError as error:
        raise TransientFailure(Reason.DEADLINE_EXCEEDED, f"stage={stage}") from error
    finally:
        if process.returncode is None:
            process.kill()
            await process.wait()
    return await process.wait(), stdout, stderr


def resample(samples: Float32Array, from_rate: int, to_rate: int) -> Float32Array:
    if from_rate == to_rate:
        return samples
    resampled = SOXR.resample(samples, from_rate, to_rate, quality="HQ")
    return np.ascontiguousarray(resampled, dtype=np.float32)


def to_mono(pcm: Pcm) -> Float32Array:
    if pcm.channels == 1:
        return pcm.samples[:, 0]
    return pcm.samples.mean(axis=1, dtype=np.float32)


def to_stereo(pcm: Pcm) -> Float32Array:
    if pcm.channels == 2:
        return pcm.samples
    return np.repeat(pcm.samples[:, :1], 2, axis=1)


def rms_dbfs(signal: Float32Array) -> float:
    if signal.size == 0:
        return 20.0 * float(np.log10(SILENCE_FLOOR))
    rms = float(np.sqrt(np.mean(np.square(signal, dtype=np.float64))))
    return 20.0 * float(np.log10(max(rms, SILENCE_FLOOR)))


def pcm16_interleaved(pcm: Pcm, seconds: float) -> Int16Array:
    head = pcm.samples[: int(seconds * pcm.sample_rate)]
    scaled = np.clip(np.round(head * PCM16_SCALE), -PCM16_SCALE, PCM16_SCALE - 1)
    return scaled.astype(np.int16).reshape(-1)


def tail(stderr: bytes) -> str:
    return stderr.decode("utf-8", "replace").strip()[-STDERR_TAIL_CHARS:]
