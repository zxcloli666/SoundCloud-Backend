from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from worker.domain.deadline import Deadline
from worker.domain.video import features

pytestmark = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="needs ffmpeg")

SECONDS = 20


def render(tmp_path: Path, name: str, source: str, extra: str = "") -> Path:
    path = tmp_path / name
    filters = f"-vf {extra}" if extra else ""
    command = (
        f"ffmpeg -v error -y -f lavfi -i {source} -t {SECONDS} {filters} -pix_fmt yuv420p {path}"
    )
    subprocess.run(command.split(), check=True)
    return path


async def measured(path: Path) -> features.Features:
    deadline = Deadline.after(120)
    info = await features.probe(path, deadline)
    return await features.measure(path, info, FakeEngines(), 180.0, deadline)


async def test_a_still_has_no_motion(tmp_path: Path) -> None:
    path = render(tmp_path, "still.mp4", "color=c=0x3366aa:s=640x360:r=25")
    result = await measured(path)
    assert result.motion_med == 0.0
    assert result.static_share > 0.99


async def test_a_busy_video_moves_and_cuts(tmp_path: Path) -> None:
    path = render(tmp_path, "busy.mp4", "testsrc2=s=640x360:r=25")
    result = await measured(path)
    assert result.motion_med > 0.1
    assert result.static_share < 0.5


def test_the_loop_score_sees_a_repeated_segment() -> None:
    rng = np.random.default_rng(0)
    segment = rng.integers(0, 255, (25, 90, 160), dtype=np.uint8)
    repeated = np.tile(segment, (6, 1, 1))
    fresh = rng.integers(0, 255, (150, 90, 160), dtype=np.uint8)
    score, period = features.loop_stats(repeated)
    assert score > 0.95
    assert period == 5.0
    assert features.loop_stats(fresh)[0] < 0.2


def test_diversity_is_zero_for_identical_vectors_and_one_for_orthogonal() -> None:
    same = np.ones((4, 8), dtype=np.float32) / np.sqrt(8)
    assert features.clip_diversity(same) == pytest.approx(0.0, abs=1e-5)
    assert features.clip_diversity(np.eye(4, dtype=np.float32)) == pytest.approx(1.0)
