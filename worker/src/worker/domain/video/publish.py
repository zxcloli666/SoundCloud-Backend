from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import aiohttp

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.observability.counters import Counters
from worker.settings import VideoSection

UPLOAD_ATTEMPTS = 2
UPLOAD_PAUSE_S = 2.0
CONNECT_TIMEOUT_S = 10.0
CAP_SHARE = 0.9
ENCODE_PRESET = "fast"
ENCODE_TIMEOUT_S = 300.0

log = logging.getLogger(__name__)


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


class Uploader:
    def __init__(
        self, session: aiohttp.ClientSession, settings: VideoSection, counters: Counters
    ) -> None:
        self._session = session
        self._settings = settings
        self._counters = counters

    async def put(self, path: Path, url: str, deadline: Deadline) -> str:
        attempt = 1
        while True:
            try:
                return await self._once(path, url, deadline)
            except (aiohttp.ClientError, TimeoutError) as error:
                self._counters.inc("video_upload_failures_total", error=type(error).__name__)
                log.warning("video upload failed", extra={"error": repr(error), "attempt": attempt})
                if attempt == UPLOAD_ATTEMPTS or deadline.expired():
                    raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, str(error)) from error
                attempt += 1
                await asyncio.sleep(UPLOAD_PAUSE_S)

    async def _once(self, path: Path, url: str, deadline: Deadline) -> str:
        timeout = aiohttp.ClientTimeout(
            total=deadline.budget(self._settings.upload_timeout_s), sock_connect=CONNECT_TIMEOUT_S
        )
        headers = {
            "Authorization": f"Bearer {self._settings.upload_token}",
            "Content-Type": "video/mp4",
        }
        with path.open("rb") as handle:
            async with self._session.put(
                url, data=handle, headers=headers, timeout=timeout
            ) as response:
                body = (await response.text()).strip()
                if response.status >= 500:
                    raise aiohttp.ClientResponseError(
                        response.request_info, response.history, status=response.status
                    )
                if response.status != 201 or not body:
                    raise TransientFailure(
                        Reason.OBJECT_STORE_UNAVAILABLE, f"upload status={response.status}"
                    )
                return body
