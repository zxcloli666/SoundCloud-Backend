from __future__ import annotations

import hashlib
import subprocess
from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import pytest

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from tests.unit.video import media
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Reason, Status
from worker.domain.ports import FrameVerdict
from worker.domain.video import sample
from worker.domain.video_vlm import LABELS, PROMPT, VideoVlmLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

MODEL = "Qwen/Qwen3-VL-2B-Instruct-GGUF/Qwen3VL-2B-Instruct-Q4_K_M.gguf@52d6c8ff"


@pytest.fixture(scope="module")
def video_bytes(tmp_path_factory: pytest.TempPathFactory) -> bytes:
    path = tmp_path_factory.mktemp("clip") / "tiny.mp4"
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi",
        "-i", "testsrc2=size=160x90:rate=10:duration=4", "-c:v", "libx264",
        "-pix_fmt", "yuv420p", str(path),
    ]  # fmt: skip
    subprocess.run(argv, check=True)
    return path.read_bytes()


class Harness:
    def __init__(
        self,
        server: FakeAudioServer,
        session: aiohttp.ClientSession,
        settings: Settings,
        work_dir: Path,
        part_bytes: int,
    ) -> None:
        self.server = server
        self.engines = FakeEngines()
        counters = Counters()
        self.lane = VideoVlmLane(
            self.engines,
            AudioSource(
                session,
                timeout_s=settings.video.download_timeout_s,
                idle_s=settings.audio.download_idle_s,
                max_bytes=1 << 20,
                counters=counters,
            ),
            Workspace(work_dir, counters),
            settings.video,
            settings.vlm,
            MODEL,
            part_bytes,
            counters,
        )

    async def run(self, path: str = "/ok.wav", **extra: object) -> Outcome:
        request = {
            "sc_track_id": "98765",
            "attempt": 2,
            "video_url": self.server.url(path),
            **extra,
        }
        return await self.lane.process(request, Deadline.after(60))


@pytest.fixture
async def harness(settings: Settings, work_dir: Path, video_bytes: bytes) -> AsyncIterator[Harness]:
    async with FakeAudioServer() as server, aiohttp.ClientSession() as session:
        server.body = video_bytes
        yield Harness(server, session, settings, work_dir, 2048)


async def test_a_loop_answer_returns_the_encoded_file_in_parts_even_from_an_mpegts_source(
    harness: Harness, settings: Settings, tmp_path: Path
) -> None:
    harness.server.body = media.make_mpegts(tmp_path / "stream.mp4", seconds=4)

    outcome = await harness.run()

    assert outcome.status is Status.OK
    blob = b"".join(outcome.parts)
    assert outcome.fields["label"] == "loop"
    assert outcome.fields["model"] == MODEL
    assert outcome.fields["scores"] == {"clip": 0.1, "loop": 0.8, "static": 0.1}
    assert outcome.fields["clip"] == {
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "parts": len(outcome.parts),
    }
    assert blob[4:8] == b"ftyp"
    out = tmp_path / "loop.mp4"
    out.write_bytes(blob)
    assert media.stream_types(out) == ["video"]
    method, call = harness.engines.calls[-1]
    side, count = settings.vlm.frame_side, settings.vlm.frames
    assert method == "classify_frames"
    assert call["prompt"] == PROMPT
    assert call["labels"] == LABELS
    assert call["frames"] == (count, side, side, 3)


async def test_a_static_answer_is_a_plain_verdict(harness: Harness) -> None:
    harness.engines.frame_verdict = FrameVerdict("static", {"static": 1.0})

    outcome = await harness.run()

    assert (outcome.status, outcome.fields["label"], outcome.parts) == (Status.OK, "static", ())


async def test_a_clip_answer_returns_the_cut_piece_in_parts(harness: Harness) -> None:
    harness.engines.frame_verdict = FrameVerdict("clip", {"clip": 0.95})

    outcome = await harness.run(offset_ms=500, duration_ms=3000)

    assert outcome.status is Status.OK
    blob = b"".join(outcome.parts)
    clip = outcome.fields["clip"]
    assert outcome.fields["label"] == "clip"
    assert clip == {
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "parts": len(outcome.parts),
    }
    assert len(outcome.parts) > 1
    assert blob[4:8] == b"ftyp"
    duration = outcome.fields["duration_ms"]
    assert isinstance(duration, int)
    assert 3000 <= duration <= 4500


@pytest.mark.parametrize("path", ["/missing", "/gone", "/forbidden"])
async def test_a_file_that_is_gone_sends_the_task_back_to_jobs(harness: Harness, path: str) -> None:
    outcome = await harness.run(path)

    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.INPUTS_GONE)
    assert harness.engines.calls == []


async def test_a_server_error_is_a_transient_failure_not_a_gone_file(harness: Harness) -> None:
    outcome = await harness.run("/error")

    assert outcome.status is Status.FAILED
    assert outcome.reason is not Reason.INPUTS_GONE


async def test_bytes_that_are_not_a_video_are_undecodable(harness: Harness) -> None:
    harness.server.body = b"not a video at all"

    outcome = await harness.run()

    assert outcome.reason is Reason.UNDECODABLE_AUDIO


async def test_a_request_without_its_url_is_invalid(harness: Harness) -> None:
    outcome = await harness.lane.process({"sc_track_id": "1", "attempt": 1}, Deadline.after(5))

    assert outcome.reason is Reason.INVALID_REQUEST


def test_the_window_follows_the_offset_and_the_track_length() -> None:
    assert sample.window(60.0, 5000, 30000) == (5.0, 30.0)
    assert sample.window(60.0, 5000, None) == (5.0, 55.0)
    assert sample.window(20.0, 5000, 30000) == (5.0, 15.0)
    assert sample.window(60.0, None, None) == (0.0, 60.0)


def test_an_offset_past_the_end_falls_back_to_the_start() -> None:
    assert sample.window(10.0, 20000, 30000) == (0.0, 10.0)
    assert sample.window(10.0, 9500, None) == (0.0, 10.0)
