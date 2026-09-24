from __future__ import annotations

import importlib
from collections.abc import Mapping
from typing import Any

import numpy as np
import torch

from worker.models import muq_compat
from worker.runtime.protocol import Arrays, BadInput, SlotSpec

SAMPLE_RATE = 24_000
MIN_WINDOW_S = 1.0
WARMUP_WINDOW_S = 30.0
EMBED = "embed"


class MuqSlot:
    def __init__(self) -> None:
        self._model: Any = None
        self._device = torch.device("cpu")
        self._dtype = torch.float32

    def load(self, spec: SlotSpec) -> None:
        muq_compat.install()
        muq = importlib.import_module("muq")
        self._device = torch.device(spec.device)
        self._dtype = torch.float16 if self._device.type == "cuda" else torch.float32
        model = muq.MuQ.from_pretrained(spec.model, revision=spec.revision)
        self._model = frozen(model, self._dtype).to(self._device)

    def warmup(self) -> None:
        silence = np.zeros((1, int(WARMUP_WINDOW_S * SAMPLE_RATE)), dtype=np.float32)
        self.invoke(EMBED, {"windows": silence}, {})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != EMBED:
            raise BadInput(f"muq has no method {method!r}")
        windows = checked_windows(arrays.get("windows"))
        return {"vectors": self._embed(windows)}, {}

    def unload(self) -> None:
        self._model = None

    def _embed(self, windows: np.ndarray) -> np.ndarray:
        batch = torch.from_numpy(windows).to(self._device, self._dtype)
        with torch.inference_mode():
            hidden = self._model(batch, output_hidden_states=True).hidden_states[1:]
            pooled = torch.stack([layer.float().mean(dim=1) for layer in hidden]).mean(dim=0)
            vectors = torch.nn.functional.normalize(pooled, dim=-1)
        return np.ascontiguousarray(vectors.cpu().numpy(), dtype=np.float32)


def checked_windows(windows: np.ndarray | None) -> np.ndarray:
    if windows is None or windows.ndim != 2 or windows.shape[0] == 0:
        raise BadInput("windows must be a non-empty [n, samples] array")
    if windows.dtype != np.float32:
        raise BadInput(f"windows dtype {windows.dtype} is not float32")
    if windows.shape[1] < MIN_WINDOW_S * SAMPLE_RATE:
        raise BadInput(f"window of {windows.shape[1]} samples is shorter than {MIN_WINDOW_S} s")
    if not np.all(np.isfinite(windows)):
        raise BadInput("windows contain non-finite samples")
    return windows


def frozen(model: Any, dtype: torch.dtype) -> Any:
    model.requires_grad_(False)
    return model.eval().to(dtype)
