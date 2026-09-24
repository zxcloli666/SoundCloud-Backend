from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from typing import Any

import numpy as np
import torch
from ctc_forced_aligner import generate_emissions, get_alignments, get_spans
from transformers import AutoModelForCTC, AutoTokenizer

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

SAMPLE_RATE = 16_000
WARMUP_S = 2.0
WINDOW_S = 30
CONTEXT_S = 2
STAR = "<star>"
FRAMES_PER_TARGET = 2


class MmsAligner:
    def __init__(self) -> None:
        self._model: torch.nn.Module | None = None
        self._tokenizer: Any = None
        self._device = "cpu"
        self._dtype = torch.float32
        self._letters: frozenset[str] = frozenset()
        self._batch = 1

    def load(self, spec: SlotSpec) -> None:
        self._device = spec.device
        self._dtype = torch.float16 if spec.device.startswith("cuda") else torch.float32
        self._model = (
            AutoModelForCTC.from_pretrained(spec.model, revision=spec.revision, dtype=self._dtype)
            .to(spec.device)
            .eval()
        )
        self._tokenizer = AutoTokenizer.from_pretrained(
            spec.model, revision=spec.revision, word_delimiter_token=None
        )
        vocabulary = self._tokenizer.get_vocab()
        self._letters = frozenset(key.lower() for key in vocabulary if len(key) == 1)
        self._batch = max(1, spec.max_batch)

    def warmup(self) -> None:
        silence = np.zeros(int(WARMUP_S * SAMPLE_RATE), dtype=np.float32)
        self.invoke("ctc_align", {"clip": silence}, {"tokens": ["hello", "world"]})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "ctc_align":
            raise BadInput(f"unknown method {method}")
        clip = arrays.get("clip")
        if clip is None or clip.ndim != 1 or clip.shape[0] < 1:
            raise BadInput("clip must be a [n] float32 array at 16 kHz")
        tokens = string_list(args, "tokens")
        if not tokens:
            raise BadInput("tokens must not be empty")
        spans, score = self._align(np.ascontiguousarray(clip, dtype=np.float32), tokens)
        return {}, {"spans": spans, "score": score}

    def unload(self) -> None:
        self._model = None
        self._tokenizer = None

    def _align(self, clip: np.ndarray, tokens: Sequence[str]) -> tuple[list[list[float]], float]:
        if self._model is None or self._tokenizer is None:
            raise RuntimeError("mms aligner is not loaded")
        wave = torch.from_numpy(clip).to(self._device, self._dtype)
        emissions, stride_ms = generate_emissions(
            self._model,
            wave,
            window_length=WINDOW_S,
            context_length=CONTEXT_S,
            batch_size=self._batch,
        )
        starred: list[str] = []
        for token in tokens:
            letters = " ".join(
                character for character in token.lower() if character in self._letters
            )
            starred.extend([STAR, letters or STAR])
        targets = sum(len(item.split()) for item in starred)
        frames = int(emissions.shape[0])
        if targets * FRAMES_PER_TARGET > frames:
            raise BadInput(
                f"{targets} ctc targets need {targets * FRAMES_PER_TARGET} > {frames} frames"
            )
        segments, frame_scores, blank = get_alignments(emissions, starred, self._tokenizer)
        spans = get_spans(starred, segments, blank)
        results: list[list[float]] = []
        log_probs: list[float] = []
        for index in range(len(tokens)):
            span = spans[2 * index + 1]
            first, last = span[0].start, span[-1].end + 1
            score = float(np.mean(frame_scores[first:last])) if last > first else -math.inf
            log_probs.append(score)
            results.append(
                [first * stride_ms / 1000.0, last * stride_ms / 1000.0, bounded(math.exp(score))]
            )
        finite = [value for value in log_probs if math.isfinite(value)]
        overall = bounded(math.exp(sum(finite) / len(finite))) if finite else 0.0
        return results, overall


def bounded(value: float) -> float:
    return min(1.0, max(0.0, value))


def string_list(args: Mapping[str, object], key: str) -> list[str]:
    value = args.get(key)
    if not isinstance(value, list | tuple):
        raise BadInput(f"{key} must be a list of strings")
    return [str(item) for item in value]
