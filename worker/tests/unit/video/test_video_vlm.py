from __future__ import annotations

import hashlib
import subprocess
from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import cv2
import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from tests.unit.video import media
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Reason, Status
from worker.domain.ports import FrameVerdict
from worker.domain.video import sample
from worker.domain.video_vlm import ANSWERS, VideoVlmLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

PROMPT = "Answer with one word: static or clip."
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
            "video-vlm",
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
            "prompt": PROMPT,
            **extra,
        }
        return await self.lane.process(request, Deadline.after(60))


@pytest.fixture
async def harness(settings: Settings, work_dir: Path, video_bytes: bytes) -> AsyncIterator[Harness]:
    async with FakeAudioServer() as server, aiohttp.ClientSession() as session:
        server.body = video_bytes
        yield Harness(server, session, settings, work_dir, 2048)


async def test_a_prompt_gets_only_the_answer_static_or_clip_without_a_file(
    harness: Harness, settings: Settings
) -> None:
    outcome = await harness.run(offset_ms=500, duration_ms=3000)

    assert outcome.status is Status.OK
    assert outcome.fields == {
        "label": "clip",
        "model": MODEL,
        "scores": {"clip": 0.8, "static": 0.2},
    }
    assert outcome.parts == ()
    method, call = harness.engines.calls[-1]
    side, count = settings.vlm.frame_side, settings.vlm.frames
    assert method == "classify_frames"
    assert call["prompt"] == PROMPT
    assert call["labels"] == ANSWERS == ("clip", "static")
    assert call["frames"] == (count, side, side, 3)


async def test_a_static_answer_is_a_plain_verdict(harness: Harness) -> None:
    harness.engines.frame_verdict = FrameVerdict("static", {"static": 1.0})

    outcome = await harness.run()

    assert (outcome.status, outcome.fields["label"], outcome.parts) == (Status.OK, "static", ())


async def test_a_known_loop_label_cuts_the_file_in_parts_even_from_an_mpegts_source(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.body = media.make_mpegts(tmp_path / "stream.mp4", seconds=4)

    outcome = await harness.run(label="loop", prompt=None)

    assert outcome.status is Status.OK
    assert harness.engines.calls == []
    blob = b"".join(outcome.parts)
    assert outcome.fields["label"] == "loop"
    assert outcome.fields["clip"] == {
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "parts": len(outcome.parts),
    }
    assert len(outcome.parts) > 1
    assert blob[4:8] == b"ftyp"
    out = tmp_path / "loop.mp4"
    out.write_bytes(blob)
    assert media.stream_types(out) == ["video"]
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


async def test_frames_mode_returns_the_model_frames_as_jpeg_files_without_the_model(
    harness: Harness, settings: Settings
) -> None:
    outcome = await harness.run(frames=True, prompt=None, offset_ms=500, duration_ms=3000)

    assert outcome.status is Status.OK
    assert harness.engines.calls == []
    assert "label" not in outcome.fields and "model" not in outcome.fields
    blob = b"".join(outcome.parts)
    slices = outcome.fields["files"]
    assert isinstance(slices, list)
    assert [item["name"] for item in slices] == [
        f"frame{index}.jpg" for index in range(settings.vlm.frames)
    ]
    assert outcome.fields["clip"] == {
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "parts": len(outcome.parts),
    }
    side = settings.vlm.frame_side
    for item in slices:
        data = blob[item["offset"] : item["offset"] + item["bytes"]]
        image = cv2.imdecode(np.frombuffer(data, dtype=np.uint8), cv2.IMREAD_COLOR)
        assert image is not None and image.shape == (side, side, 3)


async def test_a_known_clip_label_returns_the_cut_piece_without_the_model(
    harness: Harness, tmp_path: Path
) -> None:
    outcome = await harness.run(label="clip", prompt=None, offset_ms=500, duration_ms=3000)

    assert outcome.status is Status.OK
    assert harness.engines.calls == []
    assert outcome.fields["label"] == "clip"
    assert "model" not in outcome.fields and "scores" not in outcome.fields
    blob = b"".join(outcome.parts)
    assert outcome.fields["clip"]["sha256"] == hashlib.sha256(blob).hexdigest()  # type: ignore[index]
    out = tmp_path / "clip.mp4"
    out.write_bytes(blob)
    assert media.stream_types(out) == ["video"]


async def test_a_cut_with_a_pad_starts_that_much_later(harness: Harness) -> None:
    plain = await harness.run(label="clip", prompt=None)
    padded = await harness.run(label="clip", prompt=None, pad_ms=2000)

    padded_ms, plain_ms = padded.fields["duration_ms"], plain.fields["duration_ms"]
    assert isinstance(padded_ms, int) and isinstance(plain_ms, int)
    assert 1800 <= padded_ms - plain_ms <= 2200


async def test_a_known_static_label_needs_neither_the_file_nor_the_model(
    harness: Harness,
) -> None:
    outcome = await harness.run("/missing", label="static")

    assert (outcome.status, outcome.fields, outcome.parts) == (Status.OK, {"label": "static"}, ())
    assert harness.engines.calls == []


async def test_an_unknown_label_is_invalid(harness: Harness) -> None:
    outcome = await harness.run(label="dance")

    assert outcome.reason is Reason.INVALID_REQUEST


async def test_the_model_needs_the_prompt_from_the_task(harness: Harness) -> None:
    outcome = await harness.run(prompt=None)

    assert outcome.reason is Reason.INVALID_REQUEST
    assert harness.engines.calls == []


def test_the_window_follows_the_offset_and_the_track_length() -> None:
    assert sample.window(60.0, 5000, 30000) == (5.0, 30.0)
    assert sample.window(60.0, 5000, None) == (5.0, 55.0)
    assert sample.window(20.0, 5000, 30000) == (5.0, 15.0)
    assert sample.window(60.0, None, None) == (0.0, 60.0)


def test_an_offset_past_the_end_falls_back_to_the_start() -> None:
    assert sample.window(10.0, 20000, 30000) == (0.0, 10.0)
    assert sample.window(10.0, 9500, None) == (0.0, 10.0)
