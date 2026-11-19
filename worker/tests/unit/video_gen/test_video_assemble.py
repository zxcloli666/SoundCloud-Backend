from __future__ import annotations

from pathlib import Path

from tests.unit.video_gen.harness import Harness, clip, probe, sliced, tone
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, Status


def request(harness: Harness, shots: list[dict[str, object]]) -> dict[str, object]:
    return {
        "sc_track_id": "98765",
        "attempt": 2,
        "step": 9,
        "audio_url": harness.url("audio.mp3"),
        "audio_offset_ms": 1500,
        "shots": shots,
        "width": 320,
        "height": 320,
        "fps": 16,
    }


def shot(harness: Harness, name: str, trim: int, duration: int) -> dict[str, object]:
    return {"url": harness.url(name), "trim_start_ms": trim, "duration_ms": duration}


async def test_shots_and_audio_become_one_square_video_of_the_planned_length(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.files["a.mp4"] = clip(tmp_path / "a.mp4", 5.0, "480x320", 16)
    harness.server.files["b.mp4"] = clip(tmp_path / "b.mp4", 5.0, "320x320", 24)
    harness.server.files["audio.mp3"] = tone(tmp_path / "audio.mp3", 12.0)
    shots = [shot(harness, "a.mp4", 1000, 3000), shot(harness, "b.mp4", 0, 2500)]

    outcome = await harness.assemble.process(request(harness, shots), Deadline.after(120))

    assert outcome.status is Status.OK
    files = sliced(outcome)
    assert list(files) == ["video.mp4"]
    video = tmp_path / "video.mp4"
    video.write_bytes(files["video.mp4"])
    info = probe(video)
    streams = info["streams"]
    assert isinstance(streams, list)
    picture = next(item for item in streams if item["codec_type"] == "video")
    sound = next(item for item in streams if item["codec_type"] == "audio")
    assert (picture["width"], picture["height"]) == (320, 320)
    assert picture["r_frame_rate"] == "16/1"
    assert sound["codec_name"] == "aac"
    format_ = info["format"]
    assert isinstance(format_, dict)
    assert abs(float(format_["duration"]) - 5.5) < 0.15


async def test_a_shot_shorter_than_its_slot_holds_the_last_frame(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.files["a.mp4"] = clip(tmp_path / "a.mp4", 1.0, "320x320", 16)
    harness.server.files["audio.mp3"] = tone(tmp_path / "audio.mp3", 6.0)

    outcome = await harness.assemble.process(
        request(harness, [shot(harness, "a.mp4", 0, 3000)]), Deadline.after(120)
    )

    video = tmp_path / "video.mp4"
    video.write_bytes(sliced(outcome)["video.mp4"])
    format_ = probe(video)["format"]
    assert isinstance(format_, dict)
    assert abs(float(format_["duration"]) - 3.0) < 0.15


async def test_a_deleted_temp_shot_sends_the_task_back_as_inputs_gone(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.files["audio.mp3"] = tone(tmp_path / "audio.mp3", 4.0)

    outcome = await harness.assemble.process(
        request(harness, [shot(harness, "gone.mp4", 0, 2000)]), Deadline.after(60)
    )

    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.INPUTS_GONE)
    assert outcome.detail is not None and outcome.detail.startswith("shot=0")


async def test_a_broken_shot_file_is_a_tool_failure_not_a_gone_input(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.files["bad.mp4"] = b"not a video"
    harness.server.files["audio.mp3"] = tone(tmp_path / "audio.mp3", 4.0)

    outcome = await harness.assemble.process(
        request(harness, [shot(harness, "bad.mp4", 0, 2000)]), Deadline.after(60)
    )

    assert outcome.status is not Status.OK
    assert outcome.reason is not Reason.INPUTS_GONE


async def test_no_shots_is_an_invalid_request(harness: Harness) -> None:
    outcome = await harness.assemble.process(request(harness, []), Deadline.after(10))

    assert outcome.reason is Reason.INVALID_REQUEST
