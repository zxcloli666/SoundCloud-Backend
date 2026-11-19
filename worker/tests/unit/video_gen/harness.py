from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

import aiohttp

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from worker.domain.audio.source import AudioSource
from worker.domain.outcome import Outcome
from worker.domain.video_assemble import VideoAssembleLane
from worker.domain.video_keyframe import VideoKeyframeLane
from worker.domain.video_shot import VideoShotLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

PART_BYTES = 65_536


class Harness:
    def __init__(
        self,
        server: FakeAudioServer,
        session: aiohttp.ClientSession,
        settings: Settings,
        work_dir: Path,
    ) -> None:
        self.server = server
        self.engines = FakeEngines()
        counters = Counters()
        source = AudioSource(
            session, timeout_s=30.0, idle_s=10.0, max_bytes=64 << 20, counters=counters
        )
        workspace = Workspace(work_dir, counters)
        gen = settings.video_gen
        self.keyframe = VideoKeyframeLane(
            self.engines, source, workspace, gen, PART_BYTES, counters
        )
        self.shot = VideoShotLane(self.engines, source, workspace, gen, PART_BYTES, counters)
        self.assemble = VideoAssembleLane(source, source, workspace, gen, PART_BYTES, counters)

    def url(self, name: str) -> str:
        return self.server.url(f"/files/{name}")


def ffmpeg(*args: str) -> None:
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *args], check=True)


def clip(target: Path, seconds: float, size: str, rate: int) -> bytes:
    ffmpeg(
        "-f", "lavfi", "-i", f"testsrc=duration={seconds}:size={size}:rate={rate}",
        "-pix_fmt", "yuv420p", "-c:v", "libx264", "-preset", "ultrafast", str(target),
    )  # fmt: skip
    return target.read_bytes()


def tone(target: Path, seconds: float) -> bytes:
    ffmpeg(
        "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}:sample_rate=44100",
        "-c:a", "libmp3lame", "-b:a", "128k", str(target),
    )  # fmt: skip
    return target.read_bytes()


def probe(path: Path) -> dict[str, object]:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", str(path)],
        check=True,
        capture_output=True,
    )
    loaded: dict[str, object] = json.loads(out.stdout)
    return loaded


def joined(outcome: Outcome) -> bytes:
    blob = b"".join(bytes(part) for part in outcome.parts)
    clip = outcome.fields["clip"]
    assert isinstance(clip, dict)
    assert clip == {
        "bytes": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "parts": len(outcome.parts),
    }
    return blob


def sliced(outcome: Outcome) -> dict[str, bytes]:
    blob = joined(outcome)
    slices = outcome.fields["files"]
    assert isinstance(slices, list)
    return {item["name"]: blob[item["offset"] : item["offset"] + item["bytes"]] for item in slices}
