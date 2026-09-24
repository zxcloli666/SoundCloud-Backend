from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

import numpy as np
import torch
from transformers import AutoModelForTokenClassification, AutoProcessor

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

SAMPLE_RATE = 16_000
WARMUP_S = 2.0
AUDIO_PROMPT = "<|audio_start|><|audio_pad|><|audio_end|>"
TIMESTAMP = "<timestamp>"
TIMESTAMPS_PER_WORD = 2
PROCESSOR_LOADER: Any = AutoProcessor.from_pretrained


class QwenAligner:
    def __init__(self) -> None:
        self._processor: Any = None
        self._model: Any = None
        self._dtype = torch.float32

    def load(self, spec: SlotSpec) -> None:
        self._dtype = torch.bfloat16 if spec.device.startswith("cuda") else torch.float32
        self._processor = PROCESSOR_LOADER(spec.model, revision=spec.revision)
        self._model = AutoModelForTokenClassification.from_pretrained(
            spec.model, revision=spec.revision, dtype=self._dtype, device_map=spec.device
        ).eval()

    def warmup(self) -> None:
        silence = np.zeros(int(WARMUP_S * SAMPLE_RATE), dtype=np.float32)
        self.invoke("align", {"clip": silence}, {"tokens": ["hello", "world"], "language": "en"})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "align":
            raise BadInput(f"unknown method {method}")
        clip = arrays.get("clip")
        if clip is None or clip.ndim != 1 or clip.shape[0] < 1:
            raise BadInput("clip must be a [n] float32 array at 16 kHz")
        tokens = string_list(args, "tokens")
        if not tokens or any(not token for token in tokens):
            raise BadInput("tokens must be non-empty strings")
        spans, score = self._align(np.ascontiguousarray(clip, dtype=np.float32), tokens)
        return {}, {"spans": spans, "score": score}

    def unload(self) -> None:
        self._model = None
        self._processor = None

    def _align(self, clip: np.ndarray, tokens: Sequence[str]) -> tuple[list[list[float]], float]:
        processor, model = self._require()
        separator = TIMESTAMP * TIMESTAMPS_PER_WORD
        text = AUDIO_PROMPT + separator.join(tokens) + separator
        inputs = processor(
            text=[text], audio=[clip], sampling_rate=SAMPLE_RATE, return_tensors="pt"
        )
        inputs = inputs.to(model.device)
        inputs["input_features"] = inputs["input_features"].to(self._dtype)
        with torch.inference_mode():
            logits = model(**inputs).logits
        timestamp_id = int(model.config.timestamp_token_id)
        mask = inputs["input_ids"][0] == timestamp_id
        if int(mask.sum()) != len(tokens) * TIMESTAMPS_PER_WORD:
            raise BadInput("timestamp slots do not match the tokens")
        confidences = torch.softmax(logits[0][mask].float(), dim=-1).max(dim=-1).values.tolist()
        items = processor.decode_forced_alignment(
            logits=logits,
            input_ids=inputs["input_ids"],
            word_lists=[list(tokens)],
            timestamp_token_id=timestamp_id,
        )[0]
        spans = [
            [
                float(item["start_time"]),
                float(item["end_time"]),
                float((confidences[2 * index] + confidences[2 * index + 1]) / 2),
            ]
            for index, item in enumerate(items)
        ]
        return spans, float(sum(confidences) / len(confidences))

    def _require(self) -> tuple[Any, Any]:
        if self._processor is None or self._model is None:
            raise RuntimeError("qwen aligner is not loaded")
        return self._processor, self._model


def string_list(args: Mapping[str, object], key: str) -> list[str]:
    value = args.get(key)
    if not isinstance(value, list | tuple):
        raise BadInput(f"{key} must be a list of strings")
    return [str(item) for item in value]
