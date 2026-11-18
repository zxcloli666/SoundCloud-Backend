from __future__ import annotations

import numpy as np

from worker.domain.video import match
from worker.settings import Settings

RATE = match.FINGERPRINT_RATE
ITEMS = 600


def raw_track(seed: int, items: int = ITEMS) -> np.ndarray:
    return np.random.default_rng(seed).integers(0, 2**32, items, dtype=np.uint64).astype(np.uint32)


def flipped(raw: np.ndarray, rate: float, seed: int) -> np.ndarray:
    chooser = np.random.default_rng(seed)
    bits = chooser.random((raw.size, 32)) < rate
    mask = (bits * (1 << np.arange(32, dtype=np.uint64))).sum(axis=1).astype(np.uint32)
    return raw ^ mask


def melody(seconds: float, seed: int) -> np.ndarray:
    chooser = np.random.default_rng(seed)
    notes = chooser.choice([220.0, 261.63, 293.66, 329.63, 392.0, 440.0, 523.25], int(seconds * 2))
    t = np.arange(int(RATE * 0.5)) / RATE
    return np.concatenate([np.sin(2 * np.pi * note * t) for note in notes]).astype(np.float32)


def test_a_shifted_noisy_copy_is_verified_with_its_offset(settings: Settings) -> None:
    track = raw_track(1)
    shift = 31
    video = np.concatenate([raw_track(2, shift), flipped(track, 0.04, 3)])
    mono = melody(60.0, 4)
    lead = round(shift * match.ITEM_S * RATE)
    video_mono = np.concatenate([melody(10.0, 5)[:lead], mono])

    found = match.verify(track, video, mono, video_mono, settings.video)

    assert found is not None
    assert abs(found.offset_ms - shift * match.ITEM_S * 1000) < 100
    assert found.ber < 0.15
    assert found.peak >= settings.video.peak_min


def test_an_unrelated_recording_is_refused(settings: Settings) -> None:
    mono = melody(60.0, 4)
    found = match.verify(raw_track(1), raw_track(9), mono, melody(60.0, 8), settings.video)
    assert found is None


def test_a_copy_overlapping_less_than_the_minimum_is_refused(settings: Settings) -> None:
    track = raw_track(1)
    keep = int(settings.video.overlap_min_s / match.ITEM_S) - 10
    video = np.concatenate([raw_track(2, 200), track[:keep]])
    mono = melody(60.0, 4)
    assert match.verify(track, video, mono, mono, settings.video) is None


def test_a_window_decides_not_the_average() -> None:
    errors = np.concatenate([np.full(80, 0.5), np.full(60, 0.02), np.full(80, 0.5)])
    assert match.best_window(errors, 5.0, 2.5) < 0.05


def test_raw_text_parses_to_uint32() -> None:
    assert match.parse_raw("1,4294967295").tolist() == [1, 4294967295]
    assert match.parse_raw("") is None
