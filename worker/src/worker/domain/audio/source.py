from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import aiohttp

from worker.domain.deadline import Deadline
from worker.domain.outcome import Failure, PermanentFailure, Reason, TransientFailure
from worker.observability.counters import Counters

CHUNK_BYTES = 256 * 1024
NOT_FOUND_STATUSES = frozenset({404, 410})
FORBIDDEN_STATUSES = frozenset({401, 403})

log = logging.getLogger(__name__)


class AudioSource:
    def __init__(
        self,
        session: aiohttp.ClientSession,
        *,
        timeout_s: float,
        max_bytes: int,
        counters: Counters,
    ) -> None:
        self._session = session
        self._timeout_s = timeout_s
        self._max_bytes = max_bytes
        self._counters = counters

    async def fetch(self, url: str, into: Path, deadline: Deadline) -> int:
        deadline.check("download")
        try:
            return await self._stream(url, into, deadline)
        except Failure as failure:
            self._failed(failure.reason.value, failure.detail)
            raise
        except (aiohttp.ClientError, TimeoutError) as error:
            if deadline.expired():
                self._failed("deadline", repr(error))
                raise TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=download") from error
            self._failed(type(error).__name__, repr(error))
            raise TransientFailure(
                Reason.DOWNLOAD_FAILED, f"{type(error).__name__}: {error}"
            ) from error

    async def _stream(self, url: str, into: Path, deadline: Deadline) -> int:
        budget = deadline.budget(self._timeout_s)
        if budget <= 0:
            raise TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=download")
        timeout = aiohttp.ClientTimeout(total=budget)
        async with self._session.get(url, timeout=timeout) as response:
            check_status(response.status)
            declared = response.content_length
            if declared is not None and declared > self._max_bytes:
                raise too_large(declared, self._max_bytes)
            written = 0
            with into.open("wb") as handle:
                async for chunk in response.content.iter_chunked(CHUNK_BYTES):
                    written += len(chunk)
                    if written > self._max_bytes:
                        raise too_large(written, self._max_bytes)
                    await asyncio.to_thread(handle.write, chunk)
            return written

    def _failed(self, kind: str, detail: str | None) -> None:
        self._counters.inc("download_failures_total", kind=kind)
        log.warning("audio download failed", extra={"kind": kind, "detail": detail})


def check_status(status: int) -> None:
    if status in NOT_FOUND_STATUSES:
        raise PermanentFailure(Reason.AUDIO_NOT_FOUND, f"http={status}")
    if status in FORBIDDEN_STATUSES:
        raise PermanentFailure(Reason.AUDIO_FORBIDDEN, f"http={status}")
    if status >= 400:
        raise TransientFailure(Reason.DOWNLOAD_FAILED, f"http={status}")


def too_large(size: int, limit: int) -> PermanentFailure:
    return PermanentFailure(Reason.AUDIO_TOO_LARGE, f"bytes={size} limit={limit}")
