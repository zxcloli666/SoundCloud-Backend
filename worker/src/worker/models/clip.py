from __future__ import annotations

from collections.abc import Mapping

import numpy as np
import open_clip
import torch
from huggingface_hub import snapshot_download

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

ARCHITECTURE = "ViT-B-32"
WEIGHTS = "open_clip_model.safetensors"
SIDE = 224
MEAN = (0.48145466, 0.4578275, 0.40821073)
STD = (0.26862954, 0.26130258, 0.27577711)
MAX_FRAMES = 64
WARMUP_FRAMES = 2


class ClipSlot:
    def __init__(self) -> None:
        self._model: torch.nn.Module | None = None

    def load(self, spec: SlotSpec) -> None:
        folder = snapshot_download(spec.model, revision=spec.revision, local_files_only=True)
        model, _, _ = open_clip.create_model_and_transforms(
            ARCHITECTURE, pretrained=f"{folder}/{WEIGHTS}"
        )
        self._model = model.eval()

    def warmup(self) -> None:
        frames = np.zeros((WARMUP_FRAMES, SIDE, SIDE, 3), dtype=np.uint8)
        self.invoke("embed_frames", {"frames": frames}, {})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "embed_frames":
            raise BadInput(f"clip has no method {method!r}")
        frames = arrays.get("frames")
        if (
            frames is None
            or frames.dtype != np.uint8
            or frames.ndim != 4
            or frames.shape[1:] != (SIDE, SIDE, 3)
            or not 1 <= frames.shape[0] <= MAX_FRAMES
        ):
            raise BadInput(f"frames must be uint8 [n<={MAX_FRAMES}, {SIDE}, {SIDE}, 3]")
        if self._model is None:
            raise RuntimeError("clip is not loaded")
        batch = torch.from_numpy(np.ascontiguousarray(frames)).permute(0, 3, 1, 2).float() / 255.0
        mean = torch.tensor(MEAN).view(1, 3, 1, 1)
        std = torch.tensor(STD).view(1, 3, 1, 1)
        with torch.inference_mode():
            vectors = self._model.encode_image((batch - mean) / std)
            vectors = vectors / vectors.norm(dim=-1, keepdim=True)
        return {"vectors": vectors.float().numpy()}, {}

    def unload(self) -> None:
        self._model = None
