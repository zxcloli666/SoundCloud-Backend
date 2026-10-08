from __future__ import annotations

import importlib
from collections.abc import Mapping, Sequence
from typing import Any, TypeGuard

import numpy as np
import torch

from worker.models import muq_compat
from worker.models.muq import SAMPLE_RATE, WARMUP_WINDOW_S, checked_windows, frozen, windowed
from worker.observability.logging import JsonLog
from worker.runtime import memory
from worker.runtime.protocol import Arrays, BadInput, SlotSpec

EMBED_AUDIO = "embed_audio"
EMBED_TEXT = "embed_text"
WARMUP_TEXT = "warm acoustic guitar"
TOWERS = "towers"
TOWER_METHODS: Mapping[str, frozenset[str]] = {
    "both": frozenset({EMBED_AUDIO, EMBED_TEXT}),
    "audio": frozenset({EMBED_AUDIO}),
    "text": frozenset({EMBED_TEXT}),
}
DROPPED_TOWER = {"audio": "text", "text": "audio"}


class MulanSlot:
    def __init__(self) -> None:
        self._model: Any = None
        self._device = torch.device("cpu")
        self._dtype = torch.float32
        self._methods = TOWER_METHODS["both"]
        self._log = JsonLog(component="mulan")

    def load(self, spec: SlotSpec) -> None:
        towers = str(spec.options.get(TOWERS, "both"))
        if towers not in TOWER_METHODS:
            raise ValueError(f"mulan towers must be one of {sorted(TOWER_METHODS)}")
        muq_compat.install()
        muq = importlib.import_module("muq")
        self._device = torch.device(spec.device)
        self._dtype = torch.float16 if self._device.type == "cuda" else torch.float32
        model = muq.MuQMuLan.from_pretrained(spec.model, revision=spec.revision)
        dropped = DROPPED_TOWER.get(towers)
        if dropped is not None:
            before_mib = memory.rss_mib()
            setattr(model.mulan_module, dropped, None)
            memory.trim()
            self._log.info(
                "mulan_tower_dropped",
                tower=dropped,
                rss_before_mib=before_mib,
                rss_after_mib=memory.rss_mib(),
            )
        self._methods = TOWER_METHODS[towers]
        self._model = frozen(model, self._dtype).to(self._device)

    def warmup(self) -> None:
        if EMBED_AUDIO in self._methods:
            silence = np.zeros((1, int(WARMUP_WINDOW_S * SAMPLE_RATE)), dtype=np.float32)
            self.invoke(EMBED_AUDIO, {"windows": silence}, {})
        if EMBED_TEXT in self._methods:
            self.invoke(EMBED_TEXT, {}, {"texts": [WARMUP_TEXT]})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method in TOWER_METHODS["both"] - self._methods:
            raise RuntimeError(f"mulan {method} tower is not loaded in this engine")
        if method == EMBED_AUDIO:
            windows = checked_windows(arrays.get("windows"))
            return {"vectors": windowed(windows, self._device, self._audio_pass)}, {}
        if method == EMBED_TEXT:
            texts = self._checked_texts(args.get("texts"))
            return {"vectors": self._vectors(texts=texts)}, {}
        raise BadInput(f"mulan has no method {method!r}")

    def unload(self) -> None:
        self._model = None

    def _audio_pass(self, windows: np.ndarray) -> np.ndarray:
        batch = torch.from_numpy(windows).to(self._device, self._dtype)
        return self._vectors(wavs=batch)

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
