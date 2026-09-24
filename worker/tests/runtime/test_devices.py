from __future__ import annotations

import importlib.util
import subprocess
import sys

import pytest

from worker.runtime import allocator, devices
from worker.runtime.engine_main import Options, parse

TORCH_SCRIPT = """
from worker.runtime import devices
devices.configure(False, 2)
import torch
print(torch.backends.mkldnn.enabled, torch.get_num_threads(), devices.resolve("cpu"))
"""


def test_only_torch_devices_configure_torch() -> None:
    assert devices.uses_torch("cuda") and devices.uses_torch("cpu") and devices.uses_torch("auto")
    assert not devices.uses_torch("none")
    assert devices.resolve("cpu") == "cpu"


@pytest.mark.parametrize(
    ("error", "expected"),
    [
        (RuntimeError("CUDA out of memory. Tried to allocate 20.00 MiB"), True),
        (
            RuntimeError(
                "[enforce fail at alloc_cpu.cpp] DefaultCPUAllocator: can't allocate memory"
            ),
            True,
        ),
        (MemoryError(), True),
        (type("OutOfMemoryError", (RuntimeError,), {})("x"), True),
        (RuntimeError("shape mismatch"), False),
        (ValueError("out of range"), False),
    ],
)
def test_out_of_memory_detection(error: BaseException, expected: bool) -> None:
    assert allocator.is_out_of_memory(error) is expected


def test_engine_argv_round_trips() -> None:
    argv = ["--fd", "7", "--owner", "42", "--onednn", "off", "--threads", "3"]
    options = parse([*argv, "--release-after-call", "on"])
    assert options == Options(
        fd=7, owner=42, onednn=False, threads=3, release_after_call=True, oom_score_adj=900
    )


@pytest.mark.skipif(importlib.util.find_spec("torch") is None, reason="torch not installed")
def test_configure_disables_onednn_and_sets_threads_in_a_fresh_process() -> None:
    result = subprocess.run(
        [sys.executable, "-c", TORCH_SCRIPT], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.split() == ["False", "2", "cpu"]
