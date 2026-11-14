from __future__ import annotations

import shutil
import time

import numpy as np
import pytest

from worker.domain.audio import opus
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, TransientFailure

CLIP_S = 20


def voice_like(seconds: float) -> np.ndarray:
    t = np.arange(int(seconds * opus.SAMPLE_RATE)) / opus.SAMPLE_RATE
    wobble = 1.0 + 0.3 * np.sin(2 * np.pi * 3 * t)
    return (0.3 * wobble * np.sin(2 * np.pi * 180 * t)).astype(np.float32)


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="needs ffmpeg")
async def test_ffmpeg_turns_the_samples_into_an_ogg_opus_clip() -> None:
    clip = await opus.OpusEncoder().encode(voice_like(CLIP_S), Deadline.after(30))

    assert clip.startswith(opus.OGG_MAGIC)
    assert b"OpusHead" in clip[:200]
    assert 20_000 < len(clip) < CLIP_S * 48_000 // 8 * 2


def fake_ffmpeg(monkeypatch: pytest.MonkeyPatch, *argv: str) -> None:
    monkeypatch.setattr(opus, "ARGV", argv)


BROKEN = {
    "missing": ("/nonexistent/ffmpeg",),
    "failing": ("sh", "-c", "cat >/dev/null; echo 'Unknown encoder libopus' >&2; exit 1"),
    "silent": ("sh", "-c", "cat >/dev/null"),
    "not ogg": ("sh", "-c", "cat >/dev/null; printf RIFF0000WAVE"),
}


@pytest.mark.parametrize("argv", BROKEN.values(), ids=list(BROKEN))
async def test_a_broken_encoder_is_an_internal_failure(
    monkeypatch: pytest.MonkeyPatch, argv: tuple[str, ...]
) -> None:
    fake_ffmpeg(monkeypatch, *argv)

    with pytest.raises(TransientFailure) as failure:
        await opus.OpusEncoder().encode(voice_like(1), Deadline.after(30))

    assert failure.value.reason is Reason.INTERNAL_ERROR


async def test_a_failing_encoder_names_its_error(monkeypatch: pytest.MonkeyPatch) -> None:
    fake_ffmpeg(monkeypatch, *BROKEN["failing"])

    with pytest.raises(TransientFailure, match="rc=1 Unknown encoder libopus"):
        await opus.OpusEncoder().encode(voice_like(1), Deadline.after(30))


async def test_a_stuck_encoder_is_stopped_by_its_own_bound(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    fake_ffmpeg(monkeypatch, "sleep", "30")
    monkeypatch.setattr(opus, "ENCODE_S", 0.2)
    started = time.monotonic()

    with pytest.raises(TransientFailure) as failure:
        await opus.OpusEncoder().encode(voice_like(1), Deadline.after(60))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED
    assert time.monotonic() - started < 5


async def test_the_call_deadline_wins_over_a_longer_bound(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    fake_ffmpeg(monkeypatch, "sleep", "30")

    with pytest.raises(TransientFailure) as failure:
        await opus.OpusEncoder().encode(voice_like(1), Deadline.after(0.2))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED
