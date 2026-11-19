from __future__ import annotations

from pathlib import Path

import pytest

from tests.unit.video import media
from worker.domain.deadline import Deadline
from worker.domain.video import publish
from worker.settings import Settings


@pytest.fixture(scope="module")
def stream(tmp_path_factory: pytest.TempPathFactory) -> Path:
    path = tmp_path_factory.mktemp("hls") / "candidate.mp4"
    media.make_mpegts(path)
    return path


async def test_an_mpegts_candidate_with_audio_becomes_a_faststart_mp4_without_audio(
    stream: Path, settings: Settings, tmp_path: Path
) -> None:
    target = tmp_path / "out.mp4"

    await publish.encode(
        stream, target, pad_s=0.0, settings=settings.video, deadline=Deadline.after(120)
    )

    blob = target.read_bytes()
    boxes = media.top_level_boxes(blob)
    assert blob[4:8] == b"ftyp"
    assert boxes.index("moov") < boxes.index("mdat")
    assert media.stream_types(target) == ["video"]


async def test_the_encoded_file_has_a_keyframe_every_two_seconds(
    stream: Path, settings: Settings, tmp_path: Path
) -> None:
    target = tmp_path / "out.mp4"

    await publish.encode(
        stream, target, pad_s=0.0, settings=settings.video, deadline=Deadline.after(120)
    )

    assert media.keyframe_times(target) == pytest.approx([0.0, 2.0, 4.0, 6.0], abs=0.05)
