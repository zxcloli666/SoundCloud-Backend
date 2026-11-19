from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import numpy as np
import torch
from diffusers import QwenImageEditPlusPipeline
from PIL import Image

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

PIPELINE_LOADER: Any = QwenImageEditPlusPipeline.from_pretrained


class ImageEditSlot:
    def __init__(self) -> None:
        self._pipe: Any = None

    def load(self, spec: SlotSpec) -> None:
        pipe = PIPELINE_LOADER(
            spec.model, revision=spec.revision, torch_dtype=torch.bfloat16, local_files_only=True
        )
        if spec.options.get("offload") and spec.device == "cuda":
            pipe.enable_model_cpu_offload()
        else:
            pipe.to(spec.device)
        self._pipe = pipe

    def warmup(self) -> None:
        return None

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != "edit":
            raise BadInput(f"image-edit has no method {method!r}")
        image = arrays.get("image")
        if image is None or image.dtype != np.uint8 or image.ndim != 3 or image.shape[2] != 3:
            raise BadInput("image must be uint8 [h, w, 3]")
        if self._pipe is None:
            raise RuntimeError("image-edit is not loaded")
        generator = torch.Generator(device="cpu").manual_seed(int(str(args["seed"])))
        with torch.inference_mode():
            result = self._pipe(
                image=[Image.fromarray(image)],
                prompt=str(args["prompt"]),
                negative_prompt=str(args["negative_prompt"]) or " ",
                width=int(str(args["width"])),
                height=int(str(args["height"])),
                num_inference_steps=int(str(args["steps"])),
                true_cfg_scale=float(str(args["true_cfg_scale"])),
                generator=generator,
            )
        frame = np.asarray(result.images[0].convert("RGB"), dtype=np.uint8)
        return {"image": np.ascontiguousarray(frame)}, {}

    def unload(self) -> None:
        self._pipe = None
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
