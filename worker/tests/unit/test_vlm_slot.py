from __future__ import annotations

import base64
import math
from dataclasses import replace

import numpy as np
import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from worker import settings as s
from worker.models.vlm import VlmSlot, data_uri, scores_of
from worker.runtime.protocol import BadInput

LABELS = ["clip", "loop", "static"]


def top(*pairs: tuple[str, float]) -> dict[str, object]:
    entries = [{"token": token, "logprob": math.log(p)} for token, p in pairs]
    return {"content": [{"token": pairs[0][0], "top_logprobs": entries}]}


def test_scores_are_the_normalized_mass_of_the_tokens_that_start_each_label() -> None:
    scores = scores_of(top(("lo", 0.6), ("clip", 0.3), ("st", 0.05), (" ", 0.05)), LABELS)

    assert scores == pytest.approx({"clip": 0.3 / 0.95, "loop": 0.6 / 0.95, "static": 0.05 / 0.95})


def test_without_logprobs_there_are_no_scores() -> None:
    assert scores_of(None, LABELS) == {}
    assert scores_of(top(("xyz", 0.9)), LABELS) == {}


def test_a_frame_goes_out_as_a_jpeg_data_uri() -> None:
    uri = data_uri(np.full((32, 32, 3), 200, dtype=np.uint8))

    prefix = "data:image/jpeg;base64,"
    assert uri.startswith(prefix)
    assert base64.b64decode(uri[len(prefix) :])[:2] == b"\xff\xd8"


@pytest.mark.parametrize(
    "frames",
    [np.zeros((2, 8, 8), dtype=np.uint8), np.zeros((2, 8, 8, 3), dtype=np.float32)],
)
def test_bad_frames_are_refused_before_the_model_is_touched(frames: np.ndarray) -> None:
    with pytest.raises(BadInput):
        VlmSlot().invoke("classify", {"frames": frames}, {"prompt": "p", "labels": LABELS})


def test_an_unknown_method_is_refused() -> None:
    with pytest.raises(BadInput):
        VlmSlot().invoke("other", {}, {})


def settings() -> s.Settings:
    return s.load(CONFIG_DIR, {**BASE_ENV, s.PROFILE_ENV: "cpu"})


def test_the_shipped_vlm_settings_are_valid() -> None:
    assert s.validate_vlm(settings()) == []


def test_broken_vlm_settings_are_named() -> None:
    base = settings()
    broken = replace(
        base,
        vlm=replace(base.vlm, file="m.bin", sha256="x", frames=0, frame_side=100),
    )

    problems = " ".join(s.validate_vlm(broken))

    for word in ("vlm.file", "vlm.sha256", "vlm.frames", "vlm.frame_side"):
        assert word in problems
