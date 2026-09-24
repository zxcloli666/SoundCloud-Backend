from __future__ import annotations

import importlib
from collections.abc import Mapping, Sequence
from typing import Any, TypeGuard

import numpy as np
import torch

from worker.models import muq_compat
from worker.models.muq import SAMPLE_RATE, WARMUP_WINDOW_S, checked_windows, frozen
from worker.runtime.protocol import Arrays, BadInput, SlotSpec

EMBED_AUDIO = "embed_audio"
EMBED_TEXT = "embed_text"
WARMUP_TEXT = "warm acoustic guitar"


class MulanSlot:
    def __init__(self) -> None:
        self._model: Any = None
        self._device = torch.device("cpu")
        self._dtype = torch.float32

    def load(self, spec: SlotSpec) -> None:
        muq_compat.install()
        muq = importlib.import_module("muq")
        self._device = torch.device(spec.device)
        self._dtype = torch.float16 if self._device.type == "cuda" else torch.float32
        model = muq.MuQMuLan.from_pretrained(spec.model, revision=spec.revision)
        self._model = frozen(model, self._dtype).to(self._device)

    def warmup(self) -> None:
        silence = np.zeros((1, int(WARMUP_WINDOW_S * SAMPLE_RATE)), dtype=np.float32)
        self.invoke(EMBED_AUDIO, {"windows": silence}, {})
        self.invoke(EMBED_TEXT, {}, {"texts": [WARMUP_TEXT]})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method == EMBED_AUDIO:
            windows = checked_windows(arrays.get("windows"))
            batch = torch.from_numpy(windows).to(self._device, self._dtype)
            return {"vectors": self._vectors(wavs=batch)}, {}
        if method == EMBED_TEXT:
            texts = self._checked_texts(args.get("texts"))
            return {"vectors": self._vectors(texts=texts)}, {}
        raise BadInput(f"mulan has no method {method!r}")

    def unload(self) -> None:
        self._model = None

    def _vectors(self, **inputs: object) -> np.ndarray:
        with torch.inference_mode():
            latents = self._model(**inputs)
            vectors = torch.nn.functional.normalize(latents.float(), dim=-1)
        return np.ascontiguousarray(vectors.cpu().numpy(), dtype=np.float32)

    def _checked_texts(self, texts: object) -> list[str]:
        if not is_text_list(texts):
            raise BadInput("texts must be a non-empty list of strings")
        tokenizer = self._model.mulan_module.text.tokenizer
        limit = int(tokenizer.model_max_length)
        for text in texts:
            if not text.strip():
                raise BadInput("texts must not be blank")
            tokens = len(tokenizer(text)["input_ids"])
            if tokens > limit:
                raise BadInput(f"text of {tokens} tokens exceeds {limit}")
        return list(texts)


def is_text_list(value: object) -> TypeGuard[Sequence[str]]:
    return (
        isinstance(value, Sequence)
        and not isinstance(value, str)
        and len(value) > 0
        and all(isinstance(item, str) for item in value)
    )
