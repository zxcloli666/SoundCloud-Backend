from __future__ import annotations

from pathlib import Path

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason
from worker.settings import VideoSection

CAP_SHARE = 0.9
KEYFRAME_S = 2
ENCODE_PRESET = "fast"
ENCODE_TIMEOUT_S = 300.0


async def encode(
    source: Path,
    target: Path,
    *,
    pad_s: float,
    settings: VideoSection,
    deadline: Deadline,
) -> None:
    cap_s = cap_seconds(settings)
    filters = [
        f"scale=-2:'min({settings.encode_height},ih)':flags=lanczos",
    ]
    if pad_s > 0:
        filters.append(f"tpad=start_duration={pad_s:.3f}:start_mode=clone")
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-y",
        "-i", str(source), "-fpsmax", str(settings.encode_fps),
        "-t", f"{cap_s:.3f}", "-an", "-vf", ",".join(filters),
        "-c:v", "libx264", "-preset", ENCODE_PRESET, "-crf", str(settings.encode_crf),
        "-g", str(settings.encode_fps * KEYFRAME_S), "-sc_threshold", "0",
        "-force_key_frames", f"expr:gte(t,n_forced*{KEYFRAME_S})",
        "-maxrate", f"{settings.encode_maxrate_kbps}k",
        "-bufsize", f"{settings.encode_maxrate_kbps * 2}k",
        "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(target),
    ]  # fmt: skip
    budget = Deadline.after(deadline.budget(ENCODE_TIMEOUT_S), deadline.now)
    returncode, _, stderr = await decode.run_tool(argv, budget, "video-encode")
    if returncode != 0:
        raise decode.tool_failure("ffmpeg", returncode, stderr)
    if target.stat().st_size > settings.max_upload_mib << 20:
        raise PermanentFailure(Reason.AUDIO_TOO_LARGE, f"encoded>{settings.max_upload_mib}MiB")


def cap_seconds(settings: VideoSection) -> float:
    bytes_per_s = settings.encode_maxrate_kbps * 1000 / 8
    return (settings.max_upload_mib << 20) * CAP_SHARE / bytes_per_s


def split(blob: bytes, part_bytes: int) -> list[memoryview]:
    view = memoryview(blob)
    return [view[start : start + part_bytes] for start in range(0, len(view), part_bytes)]
