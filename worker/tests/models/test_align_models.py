from __future__ import annotations

import numpy as np
import pytest

from tests.models.test_sync_models import (
    SAMPLE_RATE,
    SpecBuilder,
    cuda_memory,
    slot_spec,
    speech_clip,
)
from worker.domain.lyrics import tokens as token_tools
from worker.runtime.protocol import BadInput

pytestmark = pytest.mark.models

TOLERANCE_S = 0.08
CTC_TOLERANCE_S = 0.12
ALIGNER_BUDGET_MIB = 3072
MMS_BUDGET_MIB = 2048
TRANSCRIPT = "Mr Quilter is the apostle of the middle classes and we are glad to welcome his gospel"
DOCUMENTED = {
    "Mr": (0.56, 0.80),
    "Quilter": (0.80, 1.28),
    "is": (1.28, 1.44),
    "apostle": (1.52, 2.08),
}

__all__ = ["cuda_memory", "slot_spec", "speech_clip"]


def words() -> list[str]:
    return token_tools.tokenize(TRANSCRIPT, "en")


def check_documented(spans: list[list[float]], tolerance_s: float) -> None:
    by_word = dict(zip(words(), spans, strict=True))
    for word, (start, end) in DOCUMENTED.items():
        got_start, got_end, _ = by_word[word]
        assert abs(got_start - start) <= tolerance_s, (word, got_start, start)
        assert abs(got_end - end) <= tolerance_s * 2, (word, got_end, end)
    starts = [span[0] for span in spans]
    assert starts == sorted(starts)


def test_qwen_aligner_matches_the_documented_timestamps(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    import torch

    from worker.models.qwen_align import QwenAligner

    aligner = QwenAligner()
    aligner.load(slot_spec("align"))
    aligner.warmup()
    _, result = aligner.invoke(
        "align", {"clip": speech_clip}, {"tokens": words(), "language": "en"}
    )
    spans = result["spans"]
    assert len(spans) == len(words())
    check_documented(spans, TOLERANCE_S)
    assert 0.0 <= result["score"] <= 1.0
    assert all(0.0 <= span[2] <= 1.0 for span in spans)
    peak = torch.cuda.max_memory_reserved() / 2**20
    with pytest.raises(BadInput):
        aligner.invoke("align", {"clip": speech_clip}, {"tokens": [], "language": "en"})
    aligner.unload()
    assert peak <= ALIGNER_BUDGET_MIB, peak


def test_qwen_aligner_accepts_a_padded_28s_region(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    from worker.models.qwen_align import QwenAligner

    aligner = QwenAligner()
    aligner.load(slot_spec("align"))
    long_clip = np.zeros(SAMPLE_RATE * 32, dtype=np.float32)
    long_clip[SAMPLE_RATE * 20 : SAMPLE_RATE * 20 + speech_clip.shape[0]] = speech_clip
    _, result = aligner.invoke("align", {"clip": long_clip}, {"tokens": words(), "language": "en"})
    spans = result["spans"]
    assert len(spans) == len(words())
    assert abs(spans[1][0] - (20.0 + 0.80)) <= 0.5
    aligner.unload()


def test_mms_aligner_matches_the_documented_timestamps(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    import torch

    from worker.models.mms_align import MmsAligner

    aligner = MmsAligner()
    aligner.load(slot_spec("mms"))
    aligner.warmup()
    romanized = token_tools.romanize(words(), "en")
    _, result = aligner.invoke("ctc_align", {"clip": speech_clip}, {"tokens": romanized})
    spans = result["spans"]
    assert len(spans) == len(words())
    check_documented(spans, CTC_TOLERANCE_S)
    assert 0.0 < result["score"] <= 1.0
    peak = torch.cuda.max_memory_reserved() / 2**20
    aligner.unload()
    assert peak <= MMS_BUDGET_MIB, peak


def test_mms_aligner_handles_a_full_track_in_windows(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    from worker.models.mms_align import MmsAligner

    aligner = MmsAligner()
    aligner.load(slot_spec("mms"))
    track = np.zeros(SAMPLE_RATE * 70, dtype=np.float32)
    track[SAMPLE_RATE * 5 : SAMPLE_RATE * 5 + speech_clip.shape[0]] = speech_clip
    track[SAMPLE_RATE * 45 : SAMPLE_RATE * 45 + speech_clip.shape[0]] = speech_clip
    romanized = token_tools.romanize(words() + words(), "en")
    _, result = aligner.invoke("ctc_align", {"clip": track}, {"tokens": romanized})
    spans = result["spans"]
    count = len(words())
    assert abs(spans[1][0] - (5.0 + 0.80)) <= 0.3
    assert abs(spans[count + 1][0] - (45.0 + 0.80)) <= 0.3
    aligner.unload()


def test_mms_aligner_refuses_text_longer_than_the_clip(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    from worker.models.mms_align import MmsAligner

    aligner = MmsAligner()
    aligner.load(slot_spec("mms"))
    romanized = token_tools.romanize(words() * 40, "en")
    with pytest.raises(BadInput):
        aligner.invoke("ctc_align", {"clip": speech_clip[:SAMPLE_RATE]}, {"tokens": romanized})
    aligner.unload()


def test_mms_aligner_gives_a_low_score_to_foreign_text(
    speech_clip: np.ndarray, slot_spec: SpecBuilder, cuda_memory: dict[str, float]
) -> None:
    from worker.models.mms_align import MmsAligner

    aligner = MmsAligner()
    aligner.load(slot_spec("mms"))
    matching = token_tools.romanize(words(), "en")
    foreign = token_tools.romanize(
        token_tools.tokenize("зашей мне глаза чтоб я не видел тебя", "ru"), "ru"
    )
    _, good = aligner.invoke("ctc_align", {"clip": speech_clip}, {"tokens": matching})
    _, bad = aligner.invoke("ctc_align", {"clip": speech_clip}, {"tokens": foreign})
    assert bad["score"] < good["score"]
    aligner.unload()
