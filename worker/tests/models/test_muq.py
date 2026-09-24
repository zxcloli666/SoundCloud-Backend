from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import numpy as np
import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from worker import settings as settings_module
from worker.runtime.protocol import BadInput, SlotSpec

pytestmark = pytest.mark.models

REFERENCE = Path(__file__).parent / "fixtures" / "muq_reference.npz"
MIN_REFERENCE_COSINE = 0.9999


def slot_spec(slot: str, loader: str) -> SlotSpec:
    config = settings_module.load(CONFIG_DIR, BASE_ENV).slots[slot]
    return SlotSpec(slot, loader, config.model, config.revision, "cuda", config.max_batch, 0)


def reference() -> np.lib.npyio.NpzFile:
    return np.load(REFERENCE)


def reference_windows() -> np.ndarray:
    return reference()["clip"].astype(np.float32)[None, :]


@pytest.fixture(scope="module")
def muq() -> Iterator[object]:
    import torch

    from worker.models.muq import MuqSlot

    slot = MuqSlot()
    slot.load(slot_spec("muq", "worker.models.muq:MuqSlot"))
    slot.warmup()
    yield slot
    slot.unload()
    torch.cuda.empty_cache()


def embed(slot: object, windows: np.ndarray) -> np.ndarray:
    arrays, _ = slot.invoke("embed", {"windows": windows}, {})
    return arrays["vectors"]


def test_matches_prototype_reference(muq: object) -> None:
    vector = embed(muq, reference_windows())[0]

    assert vector.shape == (1024,)
    assert float(np.dot(vector, reference()["muq"])) >= MIN_REFERENCE_COSINE


def test_batch_rows_are_unit_1024_vectors(muq: object) -> None:
    clip = reference_windows()[0]
    windows = np.stack([clip, clip[::-1].copy(), 0.3 * clip])

    vectors = embed(muq, windows)

    assert vectors.shape == (3, 1024)
    assert vectors.dtype == np.float32
    assert np.allclose(np.linalg.norm(vectors, axis=1), 1.0, atol=1e-5)
    assert np.all(np.isfinite(vectors))
    single = embed(muq, windows[1:2])[0]
    assert float(np.dot(vectors[1], single)) >= 0.999


@pytest.mark.parametrize(
    "windows",
    [
        np.zeros((1, 24_000), dtype=np.float64),
        np.zeros((1, 1_000), dtype=np.float32),
        np.zeros(48_000, dtype=np.float32),
        np.full((1, 48_000), np.nan, dtype=np.float32),
    ],
)
def test_bad_windows(muq: object, windows: np.ndarray) -> None:
    with pytest.raises(BadInput):
        embed(muq, windows)
