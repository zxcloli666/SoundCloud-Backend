from __future__ import annotations

import io
import struct
import time
import wave
from pathlib import Path

import numpy as np
import pytest

from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure


def tone(seconds: float, sample_rate: int, *, hz: float = 440.0, amplitude: float = 0.5):
    t = np.arange(int(seconds * sample_rate)) / sample_rate
    return (amplitude * np.sin(2 * np.pi * hz * t)).astype(np.float32)


def wav_bytes(samples: np.ndarray, sample_rate: int) -> bytes:
    frames = samples if samples.ndim == 2 else samples[:, None]
    pcm = np.clip(np.round(frames * 32767), -32768, 32767).astype("<i2")
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as handle:
        handle.setnchannels(frames.shape[1])
        handle.setsampwidth(2)
        handle.setframerate(sample_rate)
        handle.writeframes(pcm.tobytes())
    return buffer.getvalue()


def write_wav(path: Path, samples: np.ndarray, sample_rate: int) -> Path:
    path.write_bytes(wav_bytes(samples, sample_rate))
    return path


async def test_decode_keeps_rate_and_channels(tmp_path: Path) -> None:
    stereo = np.stack([tone(3, 44_100), tone(3, 44_100, hz=220)], axis=1)
    path = write_wav(tmp_path / "a.wav", stereo, 44_100)

    pcm = await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert pcm.sample_rate == 44_100
    assert pcm.channels == 2
    assert pcm.duration_s == pytest.approx(3.0, abs=0.01)
    assert pcm.samples.dtype == np.float32
    assert np.allclose(pcm.samples[:, 0], stereo[:, 0], atol=1e-3)


async def test_decode_mono(tmp_path: Path) -> None:
    path = write_wav(tmp_path / "m.wav", tone(2, 16_000), 16_000)

    pcm = await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert (pcm.channels, pcm.sample_rate, pcm.frames) == (1, 16_000, 32_000)


async def test_decoder_output_is_forced_to_the_probed_rate(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = write_wav(tmp_path / "sbr.wav", tone(3, 44_100), 44_100)

    async def core_rate_probe(probed: Path, deadline: Deadline) -> decode.StreamInfo:
        return decode.StreamInfo(22_050, 1)

    monkeypatch.setattr(decode, "probe", core_rate_probe)

    pcm = await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert pcm.sample_rate == 22_050
    assert pcm.duration_s == pytest.approx(3.0, abs=0.01)


async def test_garbage_is_undecodable(tmp_path: Path) -> None:
    path = tmp_path / "garbage.mp3"
    path.write_bytes(b"not audio at all" * 64)

    with pytest.raises(PermanentFailure) as caught:
        await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert caught.value.reason is Reason.UNDECODABLE_AUDIO


async def test_empty_file_is_undecodable(tmp_path: Path) -> None:
    path = tmp_path / "empty.wav"
    path.write_bytes(b"")

    with pytest.raises(PermanentFailure) as caught:
        await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert caught.value.reason is Reason.UNDECODABLE_AUDIO


def float_wav_bytes(samples: np.ndarray, sample_rate: int) -> bytes:
    data = samples.astype("<f4").tobytes()
    fmt = struct.pack("<HHIIHH", 3, 1, sample_rate, sample_rate * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt
    body += b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


async def test_float_wav_with_nan_is_undecodable(tmp_path: Path) -> None:
    samples = tone(1, 16_000)
    samples[4_000:4_100] = np.nan
    path = tmp_path / "nan.wav"
    path.write_bytes(float_wav_bytes(samples, 16_000))

    with pytest.raises(PermanentFailure) as caught:
        await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert caught.value.reason is Reason.UNDECODABLE_AUDIO
    assert "non-finite" in str(caught.value)


async def test_finite_float_wav_decodes(tmp_path: Path) -> None:
    samples = tone(1, 16_000)
    path = tmp_path / "float.wav"
    path.write_bytes(float_wav_bytes(samples, 16_000))

    pcm = await decode.decode(path, max_duration_s=450, deadline=Deadline.after(30))

    assert np.array_equal(pcm.samples[:, 0], samples)


async def test_too_long_audio_is_rejected_without_decoding_everything(tmp_path: Path) -> None:
    path = write_wav(tmp_path / "long.wav", tone(6, 8_000), 8_000)

    with pytest.raises(PermanentFailure) as caught:
        await decode.decode(path, max_duration_s=4, deadline=Deadline.after(30))

    assert caught.value.reason is Reason.AUDIO_TOO_LONG


async def test_expired_deadline_stops_before_ffmpeg(tmp_path: Path) -> None:
    path = write_wav(tmp_path / "a.wav", tone(1, 8_000), 8_000)

    with pytest.raises(TransientFailure) as caught:
        await decode.decode(path, max_duration_s=450, deadline=Deadline.after(-1))

    assert caught.value.reason is Reason.DEADLINE_EXCEEDED


async def test_tool_is_killed_at_deadline() -> None:
    started = time.monotonic()

    with pytest.raises(TransientFailure) as caught:
        await decode.run_tool(["sleep", "30"], Deadline.after(0.3), "decode")

    assert caught.value.reason is Reason.DEADLINE_EXCEEDED
    assert caught.value.detail == "stage=decode"
    assert time.monotonic() - started < 3


def test_resample_changes_rate_and_keeps_pitch() -> None:
    source = tone(2, 44_100, hz=1000)

    resampled = decode.resample(source, 44_100, 24_000)

    assert resampled.dtype == np.float32
    assert resampled.shape[0] == pytest.approx(48_000, abs=2)
    spectrum = np.abs(np.fft.rfft(resampled))
    peak_hz = np.argmax(spectrum) * 24_000 / resampled.shape[0]
    assert peak_hz == pytest.approx(1000, abs=2)


def test_resample_same_rate_is_identity() -> None:
    source = tone(1, 16_000)

    assert decode.resample(source, 16_000, 16_000) is source


def test_mono_and_stereo_views() -> None:
    left, right = tone(1, 8_000), tone(1, 8_000, hz=300)
    stereo = decode.Pcm(np.stack([left, right], axis=1), 8_000)
    mono = decode.Pcm(left[:, None], 8_000)

    assert np.allclose(decode.to_mono(stereo), (left + right) / 2, atol=1e-6)
    assert np.array_equal(decode.to_mono(mono), left)
    assert decode.to_stereo(mono).shape == (8_000, 2)
    assert decode.to_stereo(stereo) is stereo.samples


def test_rms_dbfs() -> None:
    assert decode.rms_dbfs(tone(1, 8_000, amplitude=1.0)) == pytest.approx(-3.01, abs=0.05)
    assert decode.rms_dbfs(np.zeros(100, dtype=np.float32)) == pytest.approx(-200.0)
    assert decode.rms_dbfs(np.zeros(0, dtype=np.float32)) == pytest.approx(-200.0)


def test_pcm16_interleaves_and_clips_the_head() -> None:
    samples = np.array([[1.5, -1.5], [0.5, -0.5], [0.25, 0.0]], dtype=np.float32)
    pcm = decode.Pcm(samples, 2)

    head = decode.pcm16_interleaved(pcm, 1.0)

    assert head.dtype == np.int16
    assert head.tolist() == [32767, -32768, 16384, -16384]
