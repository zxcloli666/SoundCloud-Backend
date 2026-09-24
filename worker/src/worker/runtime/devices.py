from __future__ import annotations

from typing import Any

TORCH_DEVICES = frozenset({"auto", "cuda", "cpu"})


def uses_torch(device: str) -> bool:
    return device in TORCH_DEVICES


def resolve(device: str) -> str:
    if device != "auto":
        return device
    import torch

    return "cuda" if torch.cuda.is_available() else "cpu"


def configure(onednn: bool, threads: int) -> None:
    import torch

    mkldnn: Any = torch.backends.mkldnn
    mkldnn.enabled = onednn
    if threads > 0:
        torch.set_num_threads(threads)
