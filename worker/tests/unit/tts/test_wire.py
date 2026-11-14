from __future__ import annotations

import io

import numpy as np
import pytest

from worker.models.tts import wire


def test_a_frame_survives_the_round_trip() -> None:
    samples = np.linspace(-1, 1, 480, dtype=np.float32)
    frames = [
        wire.Frame(wire.READY, 0, b"24000\ncpu"),
        wire.Frame(wire.AUDIO, 7, samples.astype("<f4").tobytes()),
        wire.Frame(wire.DONE, 7),
    ]
    stream = io.BytesIO(b"".join(wire.encode(frame) for frame in frames))

    read = [wire.read(stream), wire.read(stream), wire.read(stream), wire.read(stream)]

    assert read == [*frames, None]
    assert np.array_equal(wire.samples(frames[1].payload), samples)


def test_the_head_is_kind_id_and_length_in_little_endian() -> None:
    raw = wire.encode(wire.Frame(wire.SAY, 258, b"abc"))
    assert raw == b"S" + (258).to_bytes(4, "little") + (3).to_bytes(4, "little") + b"abc"


@pytest.mark.parametrize(
    "raw",
    [
        b"A\x01\x00",
        wire.encode(wire.Frame(wire.AUDIO, 1, b"12345678"))[:-3],
        wire.HEAD.pack(wire.AUDIO, 1, wire.MAX_PAYLOAD + 1),
    ],
    ids=["cut head", "cut payload", "oversize"],
)
def test_a_broken_stream_is_a_bad_frame(raw: bytes) -> None:
    with pytest.raises(wire.BadFrame):
        wire.read(io.BytesIO(raw))


def test_audio_must_be_whole_float32_samples() -> None:
    with pytest.raises(wire.BadFrame):
        wire.samples(b"\x00\x01\x02")


def test_text_is_cut_to_its_limit() -> None:
    assert wire.text("отказ: длинный текст\n".encode(), 5) == "отказ"
