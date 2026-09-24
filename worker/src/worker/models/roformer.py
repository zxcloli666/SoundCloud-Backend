from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

import numpy as np
import torch
import yaml
from mel_band_roformer import ensure_model_assets, get_model_from_config
from ml_collections import ConfigDict

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

CHANNELS = 2
OVERLAP = 2
WARMUP_S = 2.0


class RoformerSeparator:
    def __init__(self) -> None:
        self._model: torch.nn.Module | None = None
        self._device = "cpu"
        self._half = False
        self._chunk = 0
        self._sample_rate = 44_100
        self._max_batch = 1

    def load(self, spec: SlotSpec) -> None:
        checkpoint, config_path = ensure_model_assets(spec.model, download_missing=False)
        config = ConfigDict(yaml.safe_load(Path(config_path).read_text()))
        model = get_model_from_config("mel_band_roformer", config)
        model.load_state_dict(torch.load(checkpoint, map_location="cpu", weights_only=True))
        model.requires_grad_(False)
        self._device = spec.device
        self._half = spec.device.startswith("cuda")
        if self._half:
            model = model.half()
        self._model = model.eval().to(spec.device)
        self._chunk = int(config.inference.chunk_size)
        self._sample_rate = int(config.model.sample_rate)
        self._max_batch = max(1, spec.max_batch)

    def warmup(self) -> None:
        silence = np.zeros((CHANNELS, int(WARMUP_S * self._sample_rate)), dtype=np.float32)
        self.invoke("separate", {"mix": silence}, {})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "separate":
            raise BadInput(f"unknown method {method}")
        mix = arrays.get("mix")
        if mix is None or mix.ndim != 2 or mix.shape[0] != CHANNELS or mix.shape[1] < 1:
            raise BadInput("mix must be a [2, n] float32 array")
        vocals = self._demix(torch.from_numpy(np.ascontiguousarray(mix, dtype=np.float32)))
        return {"vocals": vocals}, {}

    def unload(self) -> None:
        self._model = None

    def _demix(self, mix: torch.Tensor) -> np.ndarray:
        model = self._require_model()
        step = self._chunk // OVERLAP
        border = self._chunk - step
        padded = mix
        if mix.shape[1] > 2 * border:
            padded = torch.nn.functional.pad(mix, (border, border), mode="reflect")
        padded = padded.to(self._device)
        window = hann(self._chunk, self._device)
        accumulated = torch.zeros(padded.shape, dtype=torch.float32, device=self._device)
        weights = torch.zeros(padded.shape[1], dtype=torch.float32, device=self._device)
        starts = list(range(0, padded.shape[1], step))
        for first in range(0, len(starts), self._max_batch):
            batch_starts = starts[first : first + self._max_batch]
            chunks = torch.stack([self._chunk_at(padded, start) for start in batch_starts])
            estimated = self._forward(model, chunks)
            for start, estimate in zip(batch_starts, estimated, strict=True):
                taken = min(self._chunk, padded.shape[1] - start)
                accumulated[:, start : start + taken] += estimate[:, :taken] * window[:taken]
                weights[start : start + taken] += window[:taken]
        vocals = accumulated / weights.clamp_min(1e-6)
        if padded.shape[1] != mix.shape[1]:
            vocals = vocals[:, border:-border]
        return vocals.float().cpu().numpy()

    def _chunk_at(self, padded: torch.Tensor, start: int) -> torch.Tensor:
        chunk = padded[:, start : start + self._chunk]
        missing = self._chunk - chunk.shape[1]
        if missing > 0:
            chunk = torch.nn.functional.pad(chunk, (0, missing))
        return chunk

    def _forward(self, model: torch.nn.Module, chunks: torch.Tensor) -> torch.Tensor:
        with (
            torch.inference_mode(),
            torch.autocast(device_type="cuda", dtype=torch.float16, enabled=self._half),
        ):
            estimated: torch.Tensor = model(chunks)
        return estimated.reshape(chunks.shape[0], CHANNELS, -1).float()

    def _require_model(self) -> torch.nn.Module:
        if self._model is None:
            raise RuntimeError("roformer is not loaded")
        return self._model


def hann(size: int, device: str) -> torch.Tensor:
    return torch.hann_window(size, periodic=False, device=device).clamp_min(1e-3)
