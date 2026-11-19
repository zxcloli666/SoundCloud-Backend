from __future__ import annotations

import asyncio
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason
from worker.domain.ports import Uint8Array
from worker.settings import VideoGenSection

STILL_QUALITY = 92
ENCODE_TIMEOUT_S = 300.0
MS = 1000.0


@dataclass(frozen=True)
class Cut:
    path: Path
    trim_start_ms: int
    duration_ms: int


def read_image(path: Path, name: str) -> Uint8Array:
    raw = np.frombuffer(path.read_bytes(), dtype=np.uint8)
    image = cv2.imdecode(raw, cv2.IMREAD_COLOR)
    if image is None:
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{name} is not an image")
    return np.ascontiguousarray(cv2.cvtColor(image, cv2.COLOR_BGR2RGB))


def png(image: Uint8Array) -> bytes:
    return encoded(".png", image, [])


def jpeg(image: Uint8Array) -> bytes:
    return encoded(".jpg", image, [cv2.IMWRITE_JPEG_QUALITY, STILL_QUALITY])


def encoded(extension: str, image: Uint8Array, params: list[int]) -> bytes:
    done, buffer = cv2.imencode(extension, cv2.cvtColor(image, cv2.COLOR_RGB2BGR), params)
    if not done:
        raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"{extension} encode failed")
    return buffer.tobytes()


def still_indices(frames: int, stills: int) -> list[int]:
    if stills <= 1:
        return [frames // 2]
    last = frames - 1
    return sorted({round(last * index / (stills - 1)) for index in range(stills)})


async def frames_to_mp4(
    frames: Uint8Array, fps: int, target: Path, settings: VideoGenSection, deadline: Deadline
) -> None:
    count, height, width, channels = frames.shape
    if channels != 3 or count < 1:
        raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"frames shape {frames.shape}")
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-y",
        "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{width}x{height}", "-r", str(fps),
        "-i", "pipe:0",
        "-c:v", "libx264", "-preset", settings.encode_preset, "-crf", str(settings.encode_crf),
        "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(target),
    ]  # fmt: skip
    feed = np.ascontiguousarray(frames).tobytes()
    await run(argv, deadline, "shot-encode", settings, target, feed)


async def assemble(
    cuts: Sequence[Cut],
    audio: Path,
    audio_offset_ms: int,
    side: tuple[int, int],
    fps: int,
    target: Path,
    settings: VideoGenSection,
    deadline: Deadline,
) -> None:
    width, height = side
    inputs: list[str] = []
    chains: list[str] = []
    for index, cut in enumerate(cuts):
        inputs += ["-i", str(cut.path)]
        start = cut.trim_start_ms / MS
        end = (cut.trim_start_ms + cut.duration_ms) / MS
        chains.append(
            f"[{index}:v]trim=start={start:.3f}:end={end:.3f},setpts=PTS-STARTPTS,"
            f"fps={fps},scale={width}:{height}:force_original_aspect_ratio=increase,"
            f"crop={width}:{height},setsar=1,"
            f"tpad=stop_mode=clone:stop_duration={cut.duration_ms / MS:.3f},"
            f"trim=duration={cut.duration_ms / MS:.3f}[v{index}]"
        )
    joined = "".join(f"[v{index}]" for index in range(len(cuts)))
    chains.append(f"{joined}concat=n={len(cuts)}:v=1:a=0[video]")
    total_s = sum(cut.duration_ms for cut in cuts) / MS
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-y", *inputs,
        "-ss", f"{audio_offset_ms / MS:.3f}", "-t", f"{total_s:.3f}", "-i", str(audio),
        "-filter_complex", ";".join(chains),
        "-map", "[video]", "-map", f"{len(cuts)}:a:0",
        "-c:v", "libx264", "-preset", settings.encode_preset, "-crf", str(settings.encode_crf),
        "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", f"{settings.audio_kbps}k",
        "-t", f"{total_s:.3f}", "-movflags", "+faststart", str(target),
    ]  # fmt: skip
    await run(argv, deadline, "assemble", settings, target)


async def run(
    argv: list[str],
    deadline: Deadline,
    stage: str,
    settings: VideoGenSection,
    target: Path,
    feed: bytes | None = None,
) -> None:
    budget = Deadline.after(deadline.budget(ENCODE_TIMEOUT_S), deadline.now)
    returncode, _, stderr = await decode.run_tool(argv, budget, stage, feed)
    if returncode != 0:
        failure = decode.tool_failure("ffmpeg", returncode, stderr)
        if isinstance(failure, PermanentFailure) and feed is not None:
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, failure.detail)
        raise failure
    size = await asyncio.to_thread(lambda: target.stat().st_size)
    if size > settings.max_output_mib << 20:
        raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"{stage}>{settings.max_output_mib}MiB")
