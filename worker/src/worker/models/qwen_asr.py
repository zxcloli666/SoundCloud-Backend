from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any, cast

import numpy as np
import torch
from transformers import (
    AutoModelForMultimodalLM,
    AutoProcessor,
    LogitsProcessor,
    LogitsProcessorList,
)
from transformers.models.qwen3_asr.processing_qwen3_asr import LANGUAGE_CODE_TO_NAME

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

SAMPLE_RATE = 16_000
WARMUP_S = 2.0
WARMUP_TOKENS = 8
ASR_TEXT_TAG = "<asr_text>"
LANGUAGE_WORD = "language"
NAME_TO_CODE = {name: code for code, name in LANGUAGE_CODE_TO_NAME.items()}
PROCESSOR_LOADER: Any = AutoProcessor.from_pretrained


class ForcedPrefix(LogitsProcessor):
    def __init__(self, prefix: Sequence[int], language_step: int) -> None:
        self._prefix = list(prefix)
        self._language_step = language_step
        self._prompt_length: int | None = None
        self.language_prob: list[float] = []
        self.language_token: list[int] = []

    def __call__(self, input_ids: torch.LongTensor, scores: torch.FloatTensor) -> torch.FloatTensor:
        if self._prompt_length is None:
            self._prompt_length = int(input_ids.shape[1])
        step = int(input_ids.shape[1]) - self._prompt_length
        if step >= len(self._prefix):
            return scores
        if step == self._language_step:
            heard = torch.softmax(scores.float(), dim=-1).max(dim=-1)
            self.language_prob = heard.values.tolist()
            self.language_token = heard.indices.tolist()
        forced = torch.full_like(scores, float("-inf"))
        forced[:, self._prefix[step]] = 0.0
        return cast(torch.FloatTensor, forced)


class QwenAsr:
    def __init__(self) -> None:
        self._processor: Any = None
        self._model: Any = None
        self._device = "cpu"
        self._dtype = torch.float32

    def load(self, spec: SlotSpec) -> None:
        self._device = spec.device
        self._dtype = torch.bfloat16 if spec.device.startswith("cuda") else torch.float32
        self._processor = PROCESSOR_LOADER(spec.model, revision=spec.revision)
        self._model = AutoModelForMultimodalLM.from_pretrained(
            spec.model, revision=spec.revision, dtype=self._dtype, device_map=spec.device
        ).eval()

    def warmup(self) -> None:
        silence = np.zeros(int(WARMUP_S * SAMPLE_RATE), dtype=np.float32)
        self.invoke(
            "draft",
            {"clip_0": silence},
            {"language": "en", "max_new_tokens": [WARMUP_TOKENS], "repetition_penalty": 1.0},
        )

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "draft":
            raise BadInput(f"unknown method {method}")
        clips = ordered_clips(arrays)
        budgets = int_list(args, "max_new_tokens")
        if len(budgets) != len(clips):
            raise BadInput("max_new_tokens must have one budget per clip")
        language = str(args.get("language"))
        if language not in LANGUAGE_CODE_TO_NAME:
            raise BadInput(f"language {language!r} is not an ASR language")
        penalty = number(args, "repetition_penalty")
        return {}, {"drafts": self._draft(clips, language, budgets, penalty)}

    def unload(self) -> None:
        self._model = None
        self._processor = None

    def _draft(
        self,
        clips: Sequence[np.ndarray],
        language: str,
        budgets: Sequence[int],
        repetition_penalty: float,
    ) -> list[dict[str, object]]:
        processor, model = self._require()
        prefix, language_step = self._prefix(language)
        forcing = ForcedPrefix(prefix, language_step)
        inputs = processor.apply_transcription_request(audio=list(clips), language=None)
        inputs = inputs.to(model.device)
        inputs["input_features"] = inputs["input_features"].to(self._dtype)
        with torch.inference_mode():
            generated = model.generate(
                **inputs,
                max_new_tokens=len(prefix) + max(budgets),
                do_sample=False,
                repetition_penalty=repetition_penalty,
                logits_processor=LogitsProcessorList([forcing]),
            )
        new_tokens = generated[:, inputs["input_ids"].shape[1] :]
        drafts: list[dict[str, object]] = []
        for row, budget, prob, token in zip(
            new_tokens, budgets, forcing.language_prob, forcing.language_token, strict=True
        ):
            parsed = processor.decode(row[: len(prefix) + budget], return_format="parsed")
            drafts.append(
                {
                    "text": str(parsed.get("transcription") or ""),
                    "language": self._language_of(token),
                    "language_prob": float(prob),
                }
            )
        return drafts

    def _prefix(self, language: str) -> tuple[list[int], int]:
        processor, _ = self._require()
        tokenizer = processor.tokenizer
        head = tokenizer.encode(LANGUAGE_WORD, add_special_tokens=False)
        name = LANGUAGE_CODE_TO_NAME[language]
        full = tokenizer.encode(f"{LANGUAGE_WORD} {name}{ASR_TEXT_TAG}", add_special_tokens=False)
        if full[: len(head)] != head or len(full) <= len(head):
            raise RuntimeError("language prefix tokenisation changed")
        return full, len(head)

    def _language_of(self, token: int) -> str | None:
        processor, _ = self._require()
        piece = processor.tokenizer.decode([token]).strip()
        if not piece:
            return None
        for name, code in NAME_TO_CODE.items():
            if name.startswith(piece):
                return code
        return None

    def _require(self) -> tuple[Any, Any]:
        if self._processor is None or self._model is None:
            raise RuntimeError("qwen asr is not loaded")
        return self._processor, self._model


def ordered_clips(arrays: Arrays) -> list[np.ndarray]:
    clips: list[np.ndarray] = []
    for index in range(len(arrays)):
        clip = arrays.get(f"clip_{index}")
        if clip is None or clip.ndim != 1 or clip.shape[0] < 1:
            raise BadInput("clips must be clip_<i> [n] float32 arrays at 16 kHz")
        clips.append(np.ascontiguousarray(clip, dtype=np.float32))
    if not clips:
        raise BadInput("no clips")
    return clips


def int_list(args: Mapping[str, object], key: str) -> list[int]:
    value = args.get(key)
    if not isinstance(value, list | tuple):
        raise BadInput(f"{key} must be a list of integers")
    return [int(item) for item in value if isinstance(item, int | float)]


def number(args: Mapping[str, object], key: str) -> float:
    value = args.get(key)
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise BadInput(f"{key} must be a number")
    return float(value)
