from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np

from tests.unit.video_gen.harness import Harness, probe, sliced
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, Status
from worker.domain.video_gen import files, media


def cover() -> bytes:
    image = np.zeros((300, 300, 3), dtype=np.uint8)
    image[:, :150] = (255, 0, 0)
    done, buffer = cv2.imencode(".jpg", image)
    assert done
    return buffer.tobytes()


def keyframe(harness: Harness) -> dict[str, object]:
    return {
        "sc_track_id": "98765",
        "attempt": 2,
        "step": 3,
        "cover_url": harness.url("cover.jpg"),
        "prompt": "the singer on a night street",
        "negative_prompt": "text",
        "width": 160,
        "height": 96,
        "seed": 7,
    }


def shot(harness: Harness) -> dict[str, object]:
    return {
        "sc_track_id": "98765",
        "attempt": 2,
        "step": 4,
        "frame_url": harness.url("frame.png"),
        "prompt": "slow push in",
        "negative_prompt": "",
        "width": 160,
        "height": 96,
        "frames": 33,
        "fps": 16,
        "stills": 4,
        "seed": 7,
    }


async def test_a_keyframe_is_drawn_from_the_cover_and_sent_back_as_png(
    harness: Harness,
) -> None:
    harness.server.files["cover.jpg"] = cover()

    outcome = await harness.keyframe.process(keyframe(harness), Deadline.after(30))

    assert outcome.status is Status.OK
    frame = sliced(outcome)["frame.png"]
    decoded = cv2.imdecode(np.frombuffer(frame, np.uint8), cv2.IMREAD_COLOR)
    assert decoded.shape == (96, 160, 3)
    method, call = harness.engines.calls[-1]
    assert method == "edit_image"
    assert call["shape"] == (300, 300, 3)
    args = call["args"]
    assert isinstance(args, dict)
    assert (args["prompt"], args["seed"], args["steps"]) == ("the singer on a night street", 7, 40)


async def test_a_keyframe_of_the_wrong_size_is_model_output_invalid(harness: Harness) -> None:
    harness.server.files["cover.jpg"] = cover()
    harness.engines.overrides["edit_image"] = lambda **_: np.zeros((10, 10, 3), np.uint8)

    outcome = await harness.keyframe.process(keyframe(harness), Deadline.after(30))

    assert outcome.reason is Reason.MODEL_OUTPUT_INVALID


async def test_a_missing_cover_is_inputs_gone(harness: Harness) -> None:
    outcome = await harness.keyframe.process(keyframe(harness), Deadline.after(30))

    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.INPUTS_GONE)


async def test_a_cover_that_is_not_an_image_is_an_invalid_request(harness: Harness) -> None:
    harness.server.files["cover.jpg"] = b"<html>"

    outcome = await harness.keyframe.process(keyframe(harness), Deadline.after(30))

    assert outcome.reason is Reason.INVALID_REQUEST


async def test_a_shot_comes_back_as_mp4_with_stills_from_start_to_end(
    harness: Harness, tmp_path: Path
) -> None:
    harness.server.files["frame.png"] = media.png(np.full((96, 160, 3), 90, np.uint8))

    outcome = await harness.shot.process(shot(harness), Deadline.after(60))

    assert outcome.status is Status.OK
    named = sliced(outcome)
    assert list(named) == ["shot.mp4", "still0.jpg", "still1.jpg", "still2.jpg", "still3.jpg"]
    video = tmp_path / "shot.mp4"
    video.write_bytes(named["shot.mp4"])
    streams = probe(video)["streams"]
    assert isinstance(streams, list)
    assert (streams[0]["width"], streams[0]["height"]) == (160, 96)
    assert int(streams[0]["nb_frames"]) == 33
    first = cv2.imdecode(np.frombuffer(named["still0.jpg"], np.uint8), cv2.IMREAD_COLOR)
    last = cv2.imdecode(np.frombuffer(named["still3.jpg"], np.uint8), cv2.IMREAD_COLOR)
    assert first.mean() < 20 < 235 < last.mean()


async def test_shot_frames_of_the_wrong_shape_are_model_output_invalid(
    harness: Harness,
) -> None:
    harness.server.files["frame.png"] = media.png(np.full((96, 160, 3), 90, np.uint8))
    harness.engines.overrides["animate_image"] = lambda **_: np.zeros((4, 8, 8, 3), np.uint8)

    outcome = await harness.shot.process(shot(harness), Deadline.after(30))

    assert outcome.reason is Reason.MODEL_OUTPUT_INVALID


def test_stills_spread_over_the_whole_shot() -> None:
    assert media.still_indices(81, 4) == [0, 27, 53, 80]
    assert media.still_indices(81, 1) == [40]


def test_files_split_into_parts_keep_their_offsets() -> None:
    outcome = files.delivered([("a", b"x" * 5), ("b", b"y" * 3)], 3)

    assert [bytes(part) for part in outcome.parts] == [b"xxx", b"xxy", b"yy"]
    assert outcome.fields["files"] == [
        {"name": "a", "offset": 0, "bytes": 5},
        {"name": "b", "offset": 5, "bytes": 3},
    ]
