from __future__ import annotations

import ctypes

import numpy as np
import pytest

from worker.models.fingerprint import LIBRARY, FingerprintSlot
from worker.runtime.protocol import BadInput, SlotSpec

SPEC = SlotSpec("cpu-tools", "worker.models.fingerprint:FingerprintSlot", "", "", "cpu", 1, 0)
RATE = 22_050


def has_chromaprint() -> bool:
    try:
        ctypes.CDLL(LIBRARY)
    except OSError:
        return False
    return True


def music(seconds: int, channels: int) -> np.ndarray:
    t = np.arange(seconds * RATE) / RATE
    notes = np.sin(2 * np.pi * (220 + 110 * np.floor(t * 2 % 4)) * t)
    frames = np.repeat((12_000 * notes)[:, None], channels, axis=1)
    return frames.astype(np.int16).reshape(-1)


def fingerprint(slot: FingerprintSlot, pcm: np.ndarray, channels: int) -> object:
    _, result = slot.invoke(
        "fingerprint", {"pcm": pcm}, {"sample_rate": RATE, "channels": channels}
    )
    return result["fingerprint"]


@pytest.fixture
def slot() -> FingerprintSlot:
    loaded = FingerprintSlot()
    loaded.load(SPEC)
    return loaded


@pytest.mark.skipif(not has_chromaprint(), reason="libchromaprint is not installed")
def test_raw_fingerprint_is_stable(slot: FingerprintSlot) -> None:
    slot.warmup()

    first = fingerprint(slot, music(30, 2), 2)
    second = fingerprint(slot, music(30, 2), 2)

    assert isinstance(first, str)
    assert first == second
    assert all(part.isdigit() for part in first.split(","))
    assert len(first) > 64


@pytest.mark.skipif(not has_chromaprint(), reason="libchromaprint is not installed")
def test_mono_and_stereo_of_one_signal_match(slot: FingerprintSlot) -> None:
    assert fingerprint(slot, music(20, 1), 1) == fingerprint(slot, music(20, 2), 2)


def test_missing_library_gives_null(monkeypatch: pytest.MonkeyPatch) -> None:
    def refuse(name: str) -> ctypes.CDLL:
        raise OSError(f"{name}: cannot open shared object file")

    monkeypatch.setattr(ctypes, "CDLL", refuse)
    slot = FingerprintSlot()
    slot.load(SPEC)

    assert fingerprint(slot, music(12, 1), 1) is None


@pytest.mark.parametrize(
    ("arrays", "args"),
    [
        ({}, {"sample_rate": RATE, "channels": 1}),
        ({"pcm": np.zeros(10, np.float32)}, {"sample_rate": RATE, "channels": 1}),
        ({"pcm": np.zeros((2, 5), np.int16)}, {"sample_rate": RATE, "channels": 1}),
        ({"pcm": np.zeros(10, np.int16)}, {"sample_rate": 0, "channels": 1}),
        ({"pcm": np.zeros(10, np.int16)}, {"sample_rate": RATE, "channels": 3}),
    ],
)
def test_bad_input(
    slot: FingerprintSlot, arrays: dict[str, np.ndarray], args: dict[str, int]
) -> None:
    with pytest.raises(BadInput):
        slot.invoke("fingerprint", arrays, args)


def test_unknown_method(slot: FingerprintSlot) -> None:
    with pytest.raises(BadInput):
        slot.invoke("embed", {}, {})
