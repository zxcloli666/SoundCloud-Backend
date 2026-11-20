from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import numpy as np
import torch
from diffusers import WanImageToVideoPipeline
from huggingface_hub import hf_hub_download
from PIL import Image

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

HIGH_ADAPTER = "lightning"
LOW_ADAPTER = "lightning_2"
PIPELINE_LOADER: Any = WanImageToVideoPipeline.from_pretrained


class ImageToVideoSlot:
    def __init__(self) -> None:
        self._pipe: Any = None

    def load(self, spec: SlotSpec) -> None:
        pipe = PIPELINE_LOADER(
            spec.model, revision=spec.revision, torch_dtype=torch.bfloat16, local_files_only=True
        )
        if spec.options.get("lightning"):
            load_lightning(pipe, spec)
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
        if method != "animate":
            raise BadInput(f"image-to-video has no method {method!r}")
        image = arrays.get("image")
        if image is None or image.dtype != np.uint8 or image.ndim != 3 or image.shape[2] != 3:
            raise BadInput("image must be uint8 [h, w, 3]")
        if self._pipe is None:
            raise RuntimeError("image-to-video is not loaded")
        width, height = int(str(args["width"])), int(str(args["height"]))
        first = (
            Image.fromarray(image).convert("RGB").resize((width, height), Image.Resampling.LANCZOS)
        )
        generator = torch.Generator(device="cpu").manual_seed(int(str(args["seed"])))
        with torch.inference_mode():
            result = self._pipe(
                image=first,
                prompt=str(args["prompt"]),
                negative_prompt=str(args["negative_prompt"]),
                width=width,
                height=height,
                num_frames=int(str(args["frames"])),
                num_inference_steps=int(str(args["steps"])),
                guidance_scale=float(str(args["guidance_scale"])),
                guidance_scale_2=float(str(args["guidance_scale_2"])),
                generator=generator,
                output_type="np",
            )
        frames = np.clip(np.asarray(result.frames[0]) * 255.0 + 0.5, 0, 255).astype(np.uint8)
        return {"frames": np.ascontiguousarray(frames)}, {}

    def unload(self) -> None:
        self._pipe = None
        if torch.cuda.is_available():
            torch.cuda.empty_cache()


def load_lightning(pipe: Any, spec: SlotSpec) -> None:
    repo, revision = str(spec.options["lora_repo"]), str(spec.options["lora_revision"])
    high = hf_hub_download(
        repo, str(spec.options["lora_high"]), revision=revision, local_files_only=True
    )
    low = hf_hub_download(
        repo, str(spec.options["lora_low"]), revision=revision, local_files_only=True
    )
    pipe.load_lora_weights(high, adapter_name=HIGH_ADAPTER)
    pipe.load_lora_weights(low, adapter_name=LOW_ADAPTER, load_into_transformer_2=True)
