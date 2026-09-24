from __future__ import annotations

import gc
import os
from collections.abc import Callable, Iterator
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf
import soxr

from worker.runtime.protocol import BadInput, SlotSpec
from worker.settings import Settings

pytestmark = pytest.mark.models

SPEECH_CLIP_ENV = "WORKER_TEST_SPEECH_CLIP"
SAMPLE_RATE = 16_000
MIX_RATE = 44_100
ROFORMER_BUDGET_MIB = 3072
ASR_BUDGET_MIB = 3072

SpecBuilder = Callable[..., SlotSpec]


@pytest.fixture(scope="session")
def speech_clip() -> np.ndarray:
    path = os.environ.get(SPEECH_CLIP_ENV)
    if not path or not Path(path).is_file():
        pytest.skip(f"needs a LibriSpeech clip in {SPEECH_CLIP_ENV}")
    audio, rate = sf.read(path, dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    if rate != SAMPLE_RATE:
        audio = soxr.resample(audio, rate, SAMPLE_RATE, quality="HQ")
    return np.ascontiguousarray(audio, dtype=np.float32)


@pytest.fixture
def slot_spec(settings: Settings) -> SpecBuilder:
    def build(name: str, device: str = "cuda") -> SlotSpec:
        slot = settings.slots[name]
        return SlotSpec(
            name=name,
            loader="",
            model=slot.model,
            revision=slot.revision,
            device=device,
            max_batch=slot.max_batch,
            max_wait_ms=slot.max_wait_ms,
        )

    return build


@pytest.fixture
def cuda_memory() -> Iterator[dict[str, float]]:
    import torch

    gc.collect()
    torch.cuda.empty_cache()
    torch.cuda.reset_peak_memory_stats()
    stats: dict[str, float] = {}
    yield stats
    stats["peak_reserved_mib"] = torch.cuda.max_memory_reserved() / 2**20
    gc.collect()
    torch.cuda.empty_cache()


def synthetic_mix(seconds: float) -> np.ndarray:
    t = np.arange(int(MIX_RATE * seconds)) / MIX_RATE
    tone = 0.3 * np.sin(2 * np.pi * 220 * t) + 0.15 * np.sin(2 * np.pi * 440 * t)
    noise = 0.02 * np.random.default_rng(0).standard_normal(t.shape)
    mono = (tone + noise).astype(np.float32)
    return np.stack([mono, mono * 0.8])


def test_roformer_separates_a_short_mix_within_budget(
    slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    import torch

    from worker.models.roformer import RoformerSeparator

    separator = RoformerSeparator()
    separator.load(slot_spec("sep"))
    separator.warmup()
    mix = synthetic_mix(30.0)
    arrays, result = separator.invoke("separate", {"mix": mix}, {})
    vocals = arrays["vocals"]
    assert vocals.shape == mix.shape and vocals.dtype == np.float32
    assert np.all(np.isfinite(vocals))
    assert float(np.abs(vocals).mean()) < float(np.abs(mix).mean())
    assert result == {}
    with pytest.raises(BadInput):
        separator.invoke("separate", {"mix": mix[0]}, {})
    peak = torch.cuda.max_memory_reserved() / 2**20
    separator.unload()
    assert peak <= ROFORMER_BUDGET_MIB, peak


def test_silero_finds_speech_and_ignores_silence(
    speech_clip: np.ndarray, slot_spec: SpecBuilder
) -> None:
    from worker.models.vad import SileroVad

    vad = SileroVad()
    vad.load(slot_spec("cpu-tools", device="cpu"))
    vad.warmup()
    args = {"threshold": 0.45, "min_speech_ms": 250, "min_silence_ms": 400, "pad_ms": 200}
    padded = np.concatenate([np.zeros(SAMPLE_RATE * 2, dtype=np.float32), speech_clip])
    _, result = vad.invoke("vad", {"vocals": padded}, args)
    spans = result["spans"]
    assert len(spans) >= 1
    assert 1.5 <= spans[0][0] <= 2.6
    assert spans[-1][1] <= padded.shape[0] / SAMPLE_RATE + 0.01
    _, silent = vad.invoke("vad", {"vocals": np.zeros(SAMPLE_RATE * 3, dtype=np.float32)}, args)
    assert silent["spans"] == []
    vad.unload()


def test_qwen_asr_drafts_the_clip_with_forced_language(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    import torch

    from worker.models.qwen_asr import QwenAsr

    asr = QwenAsr()
    asr.load(slot_spec("asr"))
    asr.warmup()
    _, result = asr.invoke(
        "draft",
        {"clip_0": speech_clip, "clip_1": speech_clip[: SAMPLE_RATE * 3]},
        {"language": "en", "max_new_tokens": [96, 64], "repetition_penalty": 1.1},
    )
    drafts = result["drafts"]
    assert len(drafts) == 2
    first = drafts[0]
    assert "quilter" in first["text"].lower() and "apostle" in first["text"].lower()
    assert first["language"] == "en"
    assert first["language_prob"] > 0.5
    assert drafts[1]["text"]
    peak = torch.cuda.max_memory_reserved() / 2**20
    with pytest.raises(BadInput):
        asr.invoke(
            "draft",
            {"clip_0": speech_clip},
            {"language": "xx", "max_new_tokens": [8], "repetition_penalty": 1.0},
        )
    asr.unload()
    assert peak <= ASR_BUDGET_MIB, peak


def test_qwen_asr_reports_its_own_language_opinion(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    from worker.models.qwen_asr import QwenAsr

    asr = QwenAsr()
    asr.load(slot_spec("asr"))
    _, result = asr.invoke(
        "draft",
        {"clip_0": speech_clip},
        {"language": "ru", "max_new_tokens": [64], "repetition_penalty": 1.1},
    )
    draft = result["drafts"][0]
    assert draft["language"] == "en"
    assert draft["language_prob"] > 0.5
    asr.unload()


def test_forced_prefix_reports_the_probability_of_the_heard_language() -> None:
    import torch

    from worker.models.qwen_asr import ForcedPrefix

    forcing = ForcedPrefix([4, 2, 1], language_step=1)
    prompt = torch.zeros((2, 4), dtype=torch.long)
    forcing(prompt, torch.zeros((2, 5), dtype=torch.float))
    scores = torch.tensor([[0.0, 0.0, 0.0, 6.0, 0.0], [0.0, 0.0, 6.0, 0.0, 0.0]])
    forced = forcing(torch.zeros((2, 5), dtype=torch.long), scores)
    assert forcing.language_token == [3, 2]
    assert forcing.language_prob[0] == forcing.language_prob[1] > 0.9
    assert forced[0].argmax().item() == 2
