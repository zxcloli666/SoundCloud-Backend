from __future__ import annotations

import socket
from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import pytest

from tests.fakes.http_audio import FakeAudioServer
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Failure, PermanentFailure, Reason, TransientFailure
from worker.observability.counters import Counters

BODY = b"RIFF" + bytes(range(256)) * 64


@pytest.fixture
async def server() -> AsyncIterator[FakeAudioServer]:
    async with FakeAudioServer(body=BODY, slow_chunk_delay_s=0.3, huge_bytes=4 << 20) as fake:
        yield fake


@pytest.fixture
async def session() -> AsyncIterator[aiohttp.ClientSession]:
    async with aiohttp.ClientSession() as client:
        yield client


def source(session: aiohttp.ClientSession, counters: Counters, **overrides: float) -> AudioSource:
    settings = {"timeout_s": 90.0, "max_bytes": 1 << 20} | overrides
    return AudioSource(
        session,
        timeout_s=settings["timeout_s"],
        max_bytes=int(settings["max_bytes"]),
        counters=counters,
    )


async def fetch_failure(audio: AudioSource, url: str, into: Path, deadline: Deadline) -> Failure:
    with pytest.raises(Failure) as caught:
        await audio.fetch(url, into, deadline)
    return caught.value


async def test_downloads_the_body(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    into = tmp_path / "audio"

    written = await source(session, Counters()).fetch(
        server.url("/ok.wav"), into, Deadline.after(30)
    )

    assert written == len(BODY)
    assert into.read_bytes() == BODY


@pytest.mark.parametrize(
    ("path", "reason"),
    [
        ("/missing", Reason.AUDIO_NOT_FOUND),
        ("/gone", Reason.AUDIO_NOT_FOUND),
        ("/forbidden", Reason.AUDIO_FORBIDDEN),
        ("/unauthorized", Reason.AUDIO_FORBIDDEN),
    ],
)
async def test_http_status_maps_to_missing(
    server: FakeAudioServer,
    session: aiohttp.ClientSession,
    tmp_path: Path,
    path: str,
    reason: Reason,
) -> None:
    counters = Counters()

    failure = await fetch_failure(
        source(session, counters), server.url(path), tmp_path / "a", Deadline.after(30)
    )

    assert isinstance(failure, PermanentFailure)
    assert failure.reason is reason
    assert counters.value("download_failures_total", kind=reason.value) == 1


async def test_server_error_is_transient(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    failure = await fetch_failure(
        source(session, Counters()), server.url("/error"), tmp_path / "a", Deadline.after(30)
    )

    assert isinstance(failure, TransientFailure)
    assert failure.reason is Reason.DOWNLOAD_FAILED
    assert failure.detail == "http=500"


async def test_declared_size_over_limit_is_too_large(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    failure = await fetch_failure(
        source(session, Counters()), server.url("/huge"), tmp_path / "a", Deadline.after(30)
    )

    assert isinstance(failure, PermanentFailure)
    assert failure.reason is Reason.AUDIO_TOO_LARGE


async def test_streamed_size_over_limit_is_too_large(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    server.slow_chunk_delay_s = 0.0
    audio = source(session, Counters(), max_bytes=6000)

    failure = await fetch_failure(audio, server.url("/slow"), tmp_path / "a", Deadline.after(30))

    assert failure.reason is Reason.AUDIO_TOO_LARGE
    assert "limit=6000" in (failure.detail or "")


async def test_slow_server_times_out_as_download_failure(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    audio = source(session, Counters(), timeout_s=0.5)

    failure = await fetch_failure(audio, server.url("/slow"), tmp_path / "a", Deadline.after(30))

    assert isinstance(failure, TransientFailure)
    assert failure.reason is Reason.DOWNLOAD_FAILED


async def test_slow_server_past_deadline_is_deadline_exceeded(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    failure = await fetch_failure(
        source(session, Counters()), server.url("/slow"), tmp_path / "a", Deadline.after(0.5)
    )

    assert failure.reason is Reason.DEADLINE_EXCEEDED
    assert failure.detail == "stage=download"


async def test_expired_deadline_does_not_request(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    failure = await fetch_failure(
        source(session, Counters()), server.url("/ok.wav"), tmp_path / "a", Deadline.after(-1)
    )

    assert failure.reason is Reason.DEADLINE_EXCEEDED
    assert server.requests == []


async def test_deadline_running_out_after_the_check_does_not_request_untimed(
    server: FakeAudioServer, session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    counters = Counters()
    moments = iter([0.0])

    def now() -> float:
        return next(moments, 10.0)

    failure = await fetch_failure(
        source(session, counters), server.url("/ok.wav"), tmp_path / "a", Deadline(10.0, now)
    )

    assert failure.reason is Reason.DEADLINE_EXCEEDED
    assert failure.detail == "stage=download"
    assert server.requests == []
    assert counters.total("download_failures_total") == 1


async def test_refused_connection_is_transient(
    session: aiohttp.ClientSession, tmp_path: Path
) -> None:
    counters = Counters()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]

    failure = await fetch_failure(
        source(session, counters),
        f"http://127.0.0.1:{port}/audio",
        tmp_path / "a",
        Deadline.after(30),
    )

    assert failure.reason is Reason.DOWNLOAD_FAILED
    assert counters.total("download_failures_total") == 1
