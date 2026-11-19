from __future__ import annotations

import base64
from dataclasses import replace

import numpy as np
import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from worker import settings as s
from worker.models.vlm import FirstStep, VlmSlot, data_uri, scores_of
from worker.runtime.protocol import BadInput

LABELS = ["clip", "loop", "static"]


STARTS = {"clip": 1, "loop": 2, "static": 3}


def test_scores_are_the_softmax_of_the_first_token_of_each_label() -> None:
    logits = np.log(np.array([0.5, 0.3, 0.15, 0.05]))

    scores = scores_of(logits, STARTS)

    assert scores == pytest.approx({"clip": 0.3 / 0.5, "loop": 0.15 / 0.5, "static": 0.05 / 0.5})


def test_without_the_first_step_there_are_no_scores() -> None:
    assert scores_of(None, STARTS) == {}
    assert scores_of(np.zeros(2), STARTS) == {}


def test_the_first_step_keeps_the_logits_it_saw_first() -> None:
    first = FirstStep()
    row = np.array([1.0, 2.0])

    assert first(np.zeros(0), row) is row
    first(np.zeros(0), np.array([9.0, 9.0]))

    assert first.logits is not None
    assert first.logits.tolist() == [1.0, 2.0]


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


def test_the_cpu_profile_runs_the_4b_q4_model_on_the_cpu_with_the_benched_settings() -> None:
    loaded = settings()
    vlm = loaded.vlm

    assert loaded.slots[s.VLM_SLOT].model == "Qwen/Qwen3-VL-4B-Instruct-GGUF"
    assert (vlm.file, vlm.mmproj) == (
        "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
        "mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf",
    )
    assert (vlm.gpu_layers, vlm.context_tokens, vlm.frames, vlm.frame_side) == (0, 4096, 8, 448)


class FakeLlama:
    def __init__(self) -> None:
        self.calls: list[dict[str, object]] = []

    def create_chat_completion(self, **kwargs: object) -> dict[str, object]:
        self.calls.append(kwargs)
        return {"choices": [{"message": {"content": "static"}}]}

    def tokenize(self, text: bytes, add_bos: bool, special: bool) -> list[int]:
        return [len(text)]


def test_the_answer_is_greedy_without_a_repeat_penalty() -> None:
    slot = VlmSlot()
    llama = FakeLlama()
    slot._llm = llama  # type: ignore[assignment]
    frames = np.zeros((2, 32, 32, 3), dtype=np.uint8)

    _, reply = slot.invoke(
        "classify", {"frames": frames}, {"prompt": "p", "labels": ["clip", "static"]}
    )

    assert reply["label"] == "static"
    call = llama.calls[0]
    assert (call["temperature"], call["repeat_penalty"]) == (0.0, 1.0)
