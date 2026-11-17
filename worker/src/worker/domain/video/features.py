from __future__ import annotations

import asyncio
import json
from dataclasses import asdict, dataclass
from itertools import pairwise
from pathlib import Path

import cv2
import numpy as np

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason
from worker.domain.ports import Engines, Float32Array, Uint8Array

MOTION_SIDE = (160, 90)
MOTION_FPS = 5
MOTION_DELTA = 12
MOTION_LAG_FRAMES = 5
QUIET_SHARE = 0.01
FLOW_SIDE = (320, 180)
FLOW_MIN_MOVING = 0.02
FLOW_MAX_PAIRS = 120
FLOW_CORNERS = 200
CLIP_FRAMES = 16
CLIP_SIDE = 224
LOOP_SIDE = (48, 27)
LOOP_MIN_LAG_S = 1.5
LOOP_MAX_LAG_S = 30.0
LOOP_BASELINE_PAIRS = 400
SCENE_SCORE = 0.35
SCENE_FPS = 10
SCENE_SIDE = 160
PROBE_FIELDS = "format=duration:stream=codec_type,width,height"

cv2.setNumThreads(1)


@dataclass(frozen=True)
class Probe:
    duration_s: float
    width: int
    height: int
    has_audio: bool


@dataclass(frozen=True)
class Features:
    duration_s: float
    motion_med: float
    motion_p90: float
    static_share: float
    quiet_time: float
    global_explained: float
    cuts_per_min: float
    loop_score: float
    loop_period_s: float
    clip_div: float

    def scores(self) -> dict[str, float]:
        return {name: round(float(value), 4) for name, value in asdict(self).items()}


async def probe(path: Path, deadline: Deadline) -> Probe:
    argv = ["ffprobe", "-v", "error", "-show_entries", PROBE_FIELDS, "-of", "json", str(path)]
    returncode, stdout, stderr = await decode.run_tool(argv, deadline, "probe-video")
    if returncode != 0:
        raise decode.tool_failure("ffprobe", returncode, stderr)
    info = json.loads(stdout or b"{}")
    streams = info.get("streams") or []
    video = next((entry for entry in streams if entry.get("codec_type") == "video"), None)
    if video is None:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "no video stream")
    duration = float((info.get("format") or {}).get("duration") or 0.0)
    if duration <= 0:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "video has no duration")
    return Probe(
        duration,
        int(video.get("width") or 0),
        int(video.get("height") or 0),
        any(entry.get("codec_type") == "audio" for entry in streams),
    )


async def measure(
    path: Path, info: Probe, engines: Engines, analysis_s: float, deadline: Deadline
) -> Features:
    window_s = min(info.duration_s, analysis_s)
    gray = await frames(path, MOTION_FPS, MOTION_SIDE, "gray", window_s, deadline)
    colour = await frames(path, 1, FLOW_SIDE, "rgb24", window_s, deadline)
    cuts = await scene_cuts(path, window_s, deadline)
    sampled = await asyncio.to_thread(evenly, colour, CLIP_FRAMES, CLIP_SIDE)
    diversity = await diversity_of(sampled, engines, deadline)
    return await asyncio.to_thread(
        assemble, info.duration_s, window_s, gray, colour, cuts, diversity
    )


def assemble(
    duration_s: float,
    window_s: float,
    gray: Uint8Array,
    colour: Uint8Array,
    cuts: int,
    diversity: float,
) -> Features:
    median, p90, static, quiet = motion_stats(gray)
    score, period = loop_stats(gray)
    return Features(
        duration_s=duration_s,
        motion_med=median,
        motion_p90=p90,
        static_share=static,
        quiet_time=quiet,
        global_explained=global_explained(colour),
        cuts_per_min=cuts / max(window_s / 60.0, 1e-6),
        loop_score=score,
        loop_period_s=period,
        clip_div=diversity,
    )


async def frames(
    path: Path,
    fps: int,
    side: tuple[int, int],
    pixel: str,
    window_s: float,
    deadline: Deadline,
) -> Uint8Array:
    width, height = side
    channels = 1 if pixel == "gray" else 3
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-t", f"{window_s:.3f}", "-i", str(path), "-an",
        "-vf", f"fps={fps},scale={width}:{height}:flags=area", "-pix_fmt", pixel,
        "-f", "rawvideo", "pipe:1",
    ]  # fmt: skip
    returncode, stdout, stderr = await decode.run_tool(argv, deadline, "video-frames")
    if returncode != 0:
        raise decode.tool_failure("ffmpeg", returncode, stderr)
    size = width * height * channels
    count = len(stdout) // size
    if count < 2:
        raise PermanentFailure(Reason.UNDECODABLE_AUDIO, f"video gave {count} frames")
    shape = (count, height, width) if channels == 1 else (count, height, width, 3)
    return np.frombuffer(stdout, dtype=np.uint8, count=count * size).reshape(shape)


async def scene_cuts(path: Path, window_s: float, deadline: Deadline) -> int:
    filters = (
        f"fps={SCENE_FPS},scale={SCENE_SIDE}:-2,"
        f"select=gt(scene\\,{SCENE_SCORE}),metadata=print:file=-"
    )
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-t", f"{window_s:.3f}", "-i", str(path), "-an",
        "-vf", filters, "-f", "null", "-",
    ]  # fmt: skip
    returncode, stdout, stderr = await decode.run_tool(argv, deadline, "video-scenes")
    if returncode != 0:
        raise decode.tool_failure("ffmpeg", returncode, stderr)
    return stdout.count(b"lavfi.scene_score")


def evenly(colour: Uint8Array, count: int, side: int) -> Uint8Array:
    picks = np.linspace(0, colour.shape[0] - 1, num=min(count, colour.shape[0])).astype(int)
    return np.stack(
        [cv2.resize(colour[index], (side, side), interpolation=cv2.INTER_AREA) for index in picks]
    )


async def diversity_of(sampled: Uint8Array, engines: Engines, deadline: Deadline) -> float:
    if sampled.shape[0] < 2:
        return 0.0
    vectors = await engines.embed_frames(sampled, deadline)
    return clip_diversity(vectors)


def clip_diversity(vectors: Float32Array) -> float:
    similarity = vectors @ vectors.T
    count = vectors.shape[0]
    off_diagonal = (similarity.sum() - np.trace(similarity)) / (count * (count - 1))
    return float(1.0 - off_diagonal)


def motion_stats(gray: Uint8Array) -> tuple[float, float, float, float]:
    signed = gray.astype(np.int16)
    lag = MOTION_LAG_FRAMES if gray.shape[0] > MOTION_LAG_FRAMES else 1
    moving = (np.abs(signed[lag:] - signed[:-lag]) > MOTION_DELTA).mean(axis=(1, 2))
    static = ((signed.max(axis=0) - signed.min(axis=0)) <= MOTION_DELTA).mean()
    return (
        float(np.median(moving)),
        float(np.percentile(moving, 90)),
        float(static),
        float((moving < QUIET_SHARE).mean()),
    )


def global_explained(colour: Uint8Array) -> float:
    gray = [cv2.cvtColor(frame, cv2.COLOR_RGB2GRAY) for frame in colour]
    explained: list[float] = []
    for before, after in pairwise(gray):
        moving = float((cv2.absdiff(before, after) > MOTION_DELTA).mean())
        if moving < FLOW_MIN_MOVING:
            continue
        residual = warped_residual(before, after)
        if residual is not None:
            explained.append(1.0 - residual / moving)
        if len(explained) >= FLOW_MAX_PAIRS:
            break
    return float(np.median(explained)) if explained else 0.0


def warped_residual(before: np.ndarray, after: np.ndarray) -> float | None:
    corners = cv2.goodFeaturesToTrack(before, FLOW_CORNERS, 0.01, 8)
    if corners is None or len(corners) < 6:
        return None
    moved, status, _ = cv2.calcOpticalFlowPyrLK(before, after, corners, None)
    keep = status.ravel() == 1
    if keep.sum() < 6:
        return None
    matrix, _ = cv2.estimateAffinePartial2D(corners[keep], moved[keep], method=cv2.RANSAC)
    if matrix is None:
        return None
    height, width = before.shape
    warped = cv2.warpAffine(before, matrix, (width, height))
    mask = cv2.warpAffine(np.full_like(before, 255), matrix, (width, height)) > 0
    if not mask.any():
        return None
    return float(((cv2.absdiff(warped, after) > MOTION_DELTA) & mask).sum() / mask.sum())


def loop_stats(gray: Uint8Array) -> tuple[float, float]:
    small = np.stack(
        [cv2.resize(frame, LOOP_SIDE, interpolation=cv2.INTER_AREA) for frame in gray]
    ).astype(np.float32)
    count = small.shape[0]
    low = round(LOOP_MIN_LAG_S * MOTION_FPS)
    high = min(round(LOOP_MAX_LAG_S * MOTION_FPS), count // 2)
    if high < low:
        return 0.0, 0.0
    chooser = np.random.default_rng(0)
    first = chooser.integers(0, count, LOOP_BASELINE_PAIRS)
    second = chooser.integers(0, count, LOOP_BASELINE_PAIRS)
    baseline = float(np.abs(small[first] - small[second]).mean())
    if baseline <= 0.0:
        return 0.0, 0.0
    gaps = [float(np.abs(small[lag:] - small[:-lag]).mean()) for lag in range(low, high + 1)]
    best = int(np.argmin(gaps))
    score = float(np.clip(1.0 - gaps[best] / baseline, 0.0, 1.0))
    return score, (low + best) / MOTION_FPS
