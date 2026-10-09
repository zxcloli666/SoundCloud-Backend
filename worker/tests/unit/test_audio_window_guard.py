from __future__ import annotations

import math
import time
from collections.abc import Iterator

import numpy as np
import pytest
import torch

from worker.models.muq import windowed
from worker.runtime.protocol import CALL_GUARD, CallExpired

WINDOWS = np.arange(8, dtype=np.float32).reshape(4, 2)


@pytest.fixture(autouse=True)
def reset_guard() -> Iterator[None]:
    yield
    CALL_GUARD.expires_at = math.inf


def test_cpu_windows_run_one_pass_each_and_keep_their_order() -> None:
    passes: list[int] = []

    def embed(batch: np.ndarray) -> np.ndarray:
        passes.append(len(batch))
        return batch * 2

    vectors = windowed(WINDOWS, torch.device("cpu"), embed)
    np.testing.assert_array_equal(vectors, WINDOWS * 2)
    assert passes == [1, 1, 1, 1]


def test_a_call_past_its_deadline_stops_between_windows() -> None:
    passes: list[int] = []

    def embed(batch: np.ndarray) -> np.ndarray:
        passes.append(len(batch))
        CALL_GUARD.expires_at = time.monotonic()
        return batch

    with pytest.raises(CallExpired):
        windowed(WINDOWS, torch.device("cpu"), embed)
    assert passes == [1]
