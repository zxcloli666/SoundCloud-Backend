from __future__ import annotations

from typing import Any

MIB = 1 << 20
OOM_MARKERS = ("out of memory", "can't allocate memory")


def is_out_of_memory(error: BaseException) -> bool:
    if isinstance(error, MemoryError) or type(error).__name__ == "OutOfMemoryError":
        return True
    text = str(error).lower()
    return any(marker in text for marker in OOM_MARKERS)


def prepare(model: Any, device: str, dtype: Any) -> Any:
    model.eval()
    model.requires_grad_(False)
    return model.to(dtype=dtype).to(device)


def release() -> None:
    import torch

    if torch.cuda.is_available():
        torch.cuda.empty_cache()


def memory_mib() -> tuple[int, int]:
    import torch

    if not torch.cuda.is_available():
        return (0, 0)
    return (int(torch.cuda.memory_reserved()) // MIB, int(torch.cuda.memory_allocated()) // MIB)
