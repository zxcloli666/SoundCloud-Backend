from __future__ import annotations

import json
import os
import sys
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, TransientFailure
from worker.observability.counters import Counters
from worker.settings import VideoSection

COOKIES_FILE = "cookies.txt"
SEARCH_TIMEOUT_S = 60.0
SOCKET_TIMEOUT_S = 20
RETRIES = 2
YOUTUBE_WATCH = "https://www.youtube.com/watch?v="
RATE_LIMIT_MARKERS = (
    "not a bot",
    "http error 429",
    "too many requests",
    "rate-limited",
    "rate limited",
)
AUTH_MARKERS = (
    "cookies are no longer valid",
    "account has been terminated",
    "login required",
    "http error 401",
    "sign in to confirm your age",
    "please sign in",
)
STDERR_TAIL = 300


class CandidateUnavailable(Exception):
    pass


@dataclass(frozen=True)
class Found:
    ref: str
    title: str
    duration_s: float | None
    url: str


@contextmanager
def cookie_file(folder: Path, text: str | None) -> Iterator[Path | None]:
    if not text:
        yield None
        return
    path = folder / COOKIES_FILE
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(text)
        yield path
    finally:
        path.unlink(missing_ok=True)


class YtDlp:
    def __init__(self, settings: VideoSection, counters: Counters) -> None:
        self._settings = settings
        self._counters = counters

    async def search(self, query: str, cookies: Path | None, deadline: Deadline) -> Sequence[Found]:
        count = self._settings.search_results
        argv = [
            *self._base(cookies),
            "--flat-playlist",
            "--dump-single-json",
            f"ytsearch{count}:{query}",
        ]
        stdout = await self._run(argv, SEARCH_TIMEOUT_S, deadline, "search")
        return parse_search(stdout)

    async def download(
        self,
        url: str,
        stem: Path,
        selector: str,
        cookies: Path | None,
        deadline: Deadline,
    ) -> Path:
        argv = [
            *self._base(cookies),
            "--no-playlist",
            "--no-part",
            "--no-continue",
            "--max-filesize",
            f"{self._settings.max_download_mib}M",
            "-f",
            selector,
            "-o",
            f"{stem}.%(ext)s",
            url,
        ]
        await self._run(argv, self._settings.download_timeout_s, deadline, "download")
        found = sorted(stem.parent.glob(f"{stem.name}.*"))
        if not found:
            raise CandidateUnavailable("yt-dlp produced no file")
        return found[0]

    def _base(self, cookies: Path | None) -> list[str]:
        argv = [
            sys.executable,
            "-m",
            "yt_dlp",
            "--ignore-config",
            "--no-warnings",
            "--no-colors",
            "--js-runtimes",
            "deno",
            "--socket-timeout",
            str(SOCKET_TIMEOUT_S),
            "--retries",
            str(RETRIES),
        ]
        if cookies is not None:
            argv += ["--cookies", str(cookies)]
        if self._settings.proxy:
            argv += ["--proxy", self._settings.proxy]
        return argv

    async def _run(
        self, argv: list[str], timeout_s: float, deadline: Deadline, stage: str
    ) -> bytes:
        budget = Deadline.after(deadline.budget(timeout_s), deadline.now)
        returncode, stdout, stderr = await decode.run_tool(argv, budget, f"yt-dlp-{stage}")
        if returncode == 0:
            return stdout
        raise self._failure(stage, stderr.decode("utf-8", "replace"))

    def _failure(self, stage: str, stderr: str) -> Exception:
        lowered = stderr.lower()
        if any(marker in lowered for marker in RATE_LIMIT_MARKERS):
            self._counters.inc("video_source_blocked_total", kind="rate_limited")
            return TransientFailure(Reason.SOURCE_RATE_LIMITED, tail(stderr))
        if any(marker in lowered for marker in AUTH_MARKERS):
            self._counters.inc("video_source_blocked_total", kind="auth_failed")
            return TransientFailure(Reason.SOURCE_AUTH_FAILED, tail(stderr))
        self._counters.inc("video_candidate_unavailable_total", stage=stage)
        return CandidateUnavailable(tail(stderr))


def parse_search(stdout: bytes) -> list[Found]:
    entries = json.loads(stdout or b"{}").get("entries") or []
    found = []
    for entry in entries:
        ref = entry.get("id")
        if not isinstance(ref, str) or not ref:
            continue
        duration = entry.get("duration")
        found.append(
            Found(
                ref=ref,
                title=str(entry.get("title") or ""),
                duration_s=float(duration) if isinstance(duration, int | float) else None,
                url=f"{YOUTUBE_WATCH}{ref}",
            )
        )
    return found


def tail(stderr: str) -> str:
    return " ".join(stderr.split())[-STDERR_TAIL:]
