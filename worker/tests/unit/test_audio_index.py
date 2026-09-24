from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from tests.unit.test_audio_decode import wav_bytes
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.index_audio import IndexAudioLane
from worker.domain.outcome import Outcome, Reason, Status, TransientFailure
from worker.domain.ports import AudioEmbeddings, EngineUnavailable
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

RATE = 22_050


def music(seconds: float, *, channels: int = 2, amplitude: float = 0.3) -> np.ndarray:
    t = np.arange(int(seconds * RATE)) / RATE
    beat = 0.5 * (1 + np.sin(2 * np.pi * 2 * t))
    left = amplitude * beat * np.sin(2 * np.pi * 330 * t)
    right = amplitude * beat * np.sin(2 * np.pi * 440 * t)
    frames = np.stack([left, right], axis=1)[:, :channels]
    return frames.astype(np.float32)


def request(url: str) -> dict[str, object]:
    return {"sc_track_id": "98765", "s3_url": url, "upload_generation": 3, "attempt": 1}


class Harness:
    def __init__(
        self,
        server: FakeAudioServer,
        session: aiohttp.ClientSession,
        engines: FakeEngines,
        settings: Settings,
        work_dir: Path,
    ) -> None:
        self.server = server
        self.engines = engines
        self.counters = Counters()
        self.work_dir = work_dir
        audio = settings.audio
        source = AudioSource(
            session,
            timeout_s=audio.download_timeout_s,
            max_bytes=audio.max_download_mib << 20,
            counters=self.counters,
        )
        self.lane = IndexAudioLane(
            engines, source, Workspace(work_dir, self.counters), audio, self.counters
        )

    async def run(self, body: bytes | None = None, path: str = "/ok.wav") -> Outcome:
        if body is not None:
            self.server.body = body
        return await self.lane.process(request(self.server.url(path)), Deadline.after(60))


@pytest.fixture
async def harness(
    engines: FakeEngines, settings: Settings, work_dir: Path
) -> AsyncIterator[Harness]:
    async with (
        FakeAudioServer() as server,
        aiohttp.ClientSession() as session,
    ):
        yield Harness(server, session, engines, settings, work_dir)


def unit(vector: object) -> float:
    return float(np.linalg.norm(np.asarray(vector, dtype=np.float64)))


async def test_indexes_a_track(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(music(100), RATE))

    assert outcome.status is Status.OK
    assert len(outcome.fields["mert"]) == 1024
    assert len(outcome.fields["clap"]) == 512
    assert unit(outcome.fields["mert"]) == pytest.approx(1.0, abs=1e-5)
    assert unit(outcome.fields["clap"]) == pytest.approx(1.0, abs=1e-5)
    assert outcome.fields["fingerprint"] == "AQADtEmUfake"
    method, kwargs = harness.engines.calls[0]
    assert method == "embed_audio"
    assert kwargs["windows"].shape == (3, 720_000)
    assert list(harness.work_dir.iterdir()) == []


async def test_fingerprint_gets_first_seconds_of_original_pcm(harness: Harness) -> None:
    await harness.run(wav_bytes(music(150), RATE))

    fingerprint = next(kwargs for name, kwargs in harness.engines.calls if name == "fingerprint")
    assert fingerprint == {"samples": 120 * RATE * 2, "sample_rate": RATE, "channels": 2}


async def test_fingerprint_is_cut_to_64_chars(harness: Harness) -> None:
    harness.engines.fingerprint_value = "1" * 500

    outcome = await harness.run(wav_bytes(music(20), RATE))

    assert outcome.fields["fingerprint"] == "1" * 64


async def test_short_clip_has_no_fingerprint(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(music(7, channels=1), RATE))

    assert outcome.status is Status.OK
    assert outcome.fields["fingerprint"] is None
    assert "fingerprint" not in [name for name, _ in harness.engines.calls]
    assert harness.engines.calls[0][1]["windows"].shape == (1, 7 * 24_000)


@pytest.mark.parametrize(
    "error",
    [
        TransientFailure(Reason.INTERNAL_ERROR, "chromaprint_feed returned 0"),
        EngineUnavailable("cpu-tools", "broken"),
    ],
)
async def test_fingerprint_failure_publishes_null(harness: Harness, error: Exception) -> None:
    harness.engines.fail("fingerprint", error)

    outcome = await harness.run(wav_bytes(music(20), RATE))

    assert outcome.status is Status.OK
    assert outcome.fields["fingerprint"] is None
    assert harness.counters.total("fingerprint_failures_total") == 1


async def test_missing_chromaprint_is_counted_and_logged(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    harness.engines.fingerprint_value = None

    outcome = await harness.run(wav_bytes(music(20), RATE))

    assert outcome.status is Status.OK
    assert outcome.fields["fingerprint"] is None
    assert harness.counters.total("fingerprint_unavailable_total") == 1
    assert "fingerprint engine returned nothing" in caplog.text


async def test_too_short_track_is_empty(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(music(3), RATE))

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.AUDIO_TOO_SHORT)
    assert harness.engines.calls == []


async def test_silent_track_is_empty(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(music(30, amplitude=0.0), RATE))

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.SILENT_AUDIO)


@pytest.mark.parametrize(
    ("path", "status", "reason"),
    [
        ("/missing", Status.MISSING, Reason.AUDIO_NOT_FOUND),
        ("/forbidden", Status.MISSING, Reason.AUDIO_FORBIDDEN),
        ("/error", Status.FAILED, Reason.DOWNLOAD_FAILED),
        ("/garbage", Status.FAILED, Reason.UNDECODABLE_AUDIO),
    ],
)
async def test_download_and_decode_outcomes(
    harness: Harness, path: str, status: Status, reason: Reason
) -> None:
    outcome = await harness.run(path=path)

    assert (outcome.status, outcome.reason) == (status, reason)
    assert list(harness.work_dir.iterdir()) == []


async def test_too_long_track_fails(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(music(451, channels=1, amplitude=0.1), 8_000))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.AUDIO_TOO_LONG)


@pytest.mark.parametrize(
    "vectors",
    [
        AudioEmbeddings(np.ones((3, 1000), np.float32), np.ones((3, 512), np.float32)),
        AudioEmbeddings(np.ones((3, 1024), np.float32), np.ones((3, 512), np.float32)),
        AudioEmbeddings(
            np.full((3, 1024), np.nan, np.float32), np.full((3, 512), 1 / np.sqrt(512), np.float32)
        ),
    ],
)
async def test_invalid_model_output_fails_deterministically(
    harness: Harness, vectors: AudioEmbeddings
) -> None:
    harness.engines.overrides["embed_audio"] = lambda **_: vectors

    outcome = await harness.run(wav_bytes(music(100), RATE))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.MODEL_OUTPUT_INVALID)
    assert harness.counters.total("model_output_invalid_total") == 1


async def test_unavailable_engine_is_transient(harness: Harness) -> None:
    harness.engines.fail("embed_audio", EngineUnavailable("muq", "restarting"))

    outcome = await harness.run(wav_bytes(music(20), RATE))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.ENGINE_CRASHED)
    assert harness.counters.value("lane_engine_unavailable_total", lane="audio", slot="muq") == 1


async def test_unexpected_error_is_internal(harness: Harness) -> None:
    harness.engines.fail("embed_audio", ValueError("broken shape"))

    outcome = await harness.run(wav_bytes(music(20), RATE))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INTERNAL_ERROR)
    assert outcome.detail == "ValueError: broken shape"
    assert harness.counters.total("lane_internal_errors_total") == 1


async def test_expired_deadline(harness: Harness) -> None:
    harness.server.body = wav_bytes(music(20), RATE)
    url = harness.server.url("/ok.wav")

    outcome = await harness.lane.process(request(url), Deadline.after(-1))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)


async def test_malformed_request_is_invalid(harness: Harness) -> None:
    outcome = await harness.lane.process({"sc_track_id": 5}, Deadline.after(60))

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
