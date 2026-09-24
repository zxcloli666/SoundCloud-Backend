from __future__ import annotations

from collections.abc import Mapping

import numpy as np
import torch
from silero_vad import get_speech_timestamps, load_silero_vad

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

SAMPLE_RATE = 16_000
WARMUP_S = 1.0


class SileroVad:
    def __init__(self) -> None:
        self._model: torch.nn.Module | None = None

    def load(self, spec: SlotSpec) -> None:
        self._model = load_silero_vad()

    def warmup(self) -> None:
        silence = np.zeros(int(WARMUP_S * SAMPLE_RATE), dtype=np.float32)
        self.invoke(
            "vad",
            {"vocals": silence},
            {"threshold": 0.5, "min_speech_ms": 250, "min_silence_ms": 100, "pad_ms": 30},
        )

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "vad":
            raise BadInput(f"unknown method {method}")
        vocals = arrays.get("vocals")
        if vocals is None or vocals.ndim != 1 or vocals.shape[0] < 1:
            raise BadInput("vocals must be a [n] float32 array at 16 kHz")
        if self._model is None:
            raise RuntimeError("silero is not loaded")
        with torch.inference_mode():
            spans = get_speech_timestamps(
                torch.from_numpy(np.ascontiguousarray(vocals, dtype=np.float32)),
                self._model,
                threshold=number(args, "threshold"),
                sampling_rate=SAMPLE_RATE,
                min_speech_duration_ms=int(number(args, "min_speech_ms")),
                min_silence_duration_ms=int(number(args, "min_silence_ms")),
                speech_pad_ms=int(number(args, "pad_ms")),
                return_seconds=True,
            )
        return {}, {"spans": [[float(span["start"]), float(span["end"])] for span in spans]}

    def unload(self) -> None:
        self._model = None


def number(args: Mapping[str, object], key: str) -> float:
    value = args.get(key)
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise BadInput(f"{key} must be a number")
    return float(value)
