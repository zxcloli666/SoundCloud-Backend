from __future__ import annotations

import math
import os
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

from tests.unit.tts import fake_qwen
from tests.unit.tts.engine_home import ENGINE, OPTIONS, SPOKEN, install, journal, voice
from worker.fetch_models import TTS_HOME_ENV
from worker.models.tts import process, slot
from worker.models.tts.slot import TtsSlot
from worker.runtime.protocol import CALL_GUARD, BadInput, CallExpired, SlotSpec
from worker.settings import TTS_DEVICES, TTS_METHOD, TTS_SLOT

TEXT = "Ставлю на паузу."


@pytest.fixture(autouse=True)
def open_deadline() -> Iterator[None]:
    CALL_GUARD.expires_at = math.inf
    yield
    CALL_GUARD.expires_at = math.inf


@pytest.fixture
def home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv(TTS_HOME_ENV, str(tmp_path))
    monkeypatch.delenv(process.LIBRARY_PATH, raising=False)
    return install(tmp_path)


@pytest.fixture
def tts(home: Path) -> Iterator[TtsSlot]:
    loaded = TtsSlot()
    loaded.load(spec())
    yield loaded
    loaded.unload()


def spec(device: str = TTS_DEVICES["cpu"]) -> SlotSpec:
    return SlotSpec(TTS_SLOT, "", "", "", device, 1, 0, OPTIONS)


def cuda_wheels(root: Path, *packages: str) -> list[Path]:
    folders = [root / "nvidia" / package / "lib" for package in packages]
    for folder in folders:
        folder.mkdir(parents=True)
    return folders


def say(tts: TtsSlot, name: str = "loli", text: str = TEXT) -> tuple[int, dict[str, object]]:
    arrays, info = tts.invoke(TTS_METHOD, {}, {"voice": name, "text": text})
    return arrays["audio"].size, dict(info)


def test_a_line_is_spoken_by_a_voice_process_started_in_the_engine_folder(
    tts: TtsSlot, home: Path
) -> None:
    spoken, info = say(tts)

    assert spoken == SPOKEN
    assert info["loaded"] is True
    [start] = journal(home, "start")
    assert start["argv"] == [str(home / "loli.gguf"), str(home / "codec.gguf"), "cpu"]
    assert start["cwd"] == str(home / ENGINE)
    assert start["library_path"] == str(home / ENGINE)
    [line] = journal(home, "say")
    assert line["payload"] == f"1\n{slot.max_tokens(TEXT)}\nloli\nauto\n{TEXT}"


def test_the_next_line_of_the_same_voice_reuses_the_process(tts: TtsSlot, home: Path) -> None:
    say(tts)
    spoken, info = say(tts)

    assert spoken == SPOKEN
    assert info == {"loaded": False, "load_s": 0.0}
    assert len(journal(home, "start")) == 1
    assert [line["id"] for line in journal(home, "say")] == [1, 2]


def test_another_voice_replaces_the_running_process(tts: TtsSlot, home: Path) -> None:
    say(tts, "loli")
    spoken, info = say(tts, "moriarty")

    assert spoken == SPOKEN and info["loaded"] is True
    assert [start["argv"][0] for start in journal(home, "start")] == [
        str(home / "loli.gguf"),
        str(home / "moriarty.gguf"),
    ]
    assert len(journal(home, "exit")) == 1
    assert journal(home, "say")[-1]["payload"].split("\n")[2] == "moriarty"


def test_unload_stops_the_voice_process(tts: TtsSlot, home: Path) -> None:
    say(tts)
    tts.unload()
    tts.unload()

    assert len(journal(home, "exit")) == 1


def test_the_deadline_cancels_the_line_and_keeps_the_voice(tts: TtsSlot, home: Path) -> None:
    voice(home, "loli", "slow")
    say(tts)
    CALL_GUARD.expires_at = time.monotonic() + 0.3

    with pytest.raises(CallExpired):
        say(tts)

    CALL_GUARD.expires_at = math.inf
    spoken, info = say(tts)
    assert spoken == fake_qwen.CHUNK * fake_qwen.SLOW_CHUNKS
    assert info["loaded"] is False
    assert [cancel["id"] for cancel in journal(home, "cancel")] == [2]


def test_a_stalled_voice_is_killed_and_the_next_line_starts_a_new_one(
    tts: TtsSlot, home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    voice(home, "loli", "stall")
    monkeypatch.setattr(process, "STALL_S", 0.3)

    with pytest.raises(RuntimeError, match="stalled"):
        say(tts)
    voice(home, "loli", "speak")
    spoken, info = say(tts)

    assert spoken == SPOKEN and info["loaded"] is True
    assert len(journal(home, "start")) == 2


def test_a_voice_that_dies_mid_line_fails_with_its_exit_code(tts: TtsSlot, home: Path) -> None:
    voice(home, "loli", "die")

    with pytest.raises(RuntimeError, match=r"died.*exit code 3"):
        say(tts)


def test_a_refused_line_keeps_the_voice(tts: TtsSlot, home: Path) -> None:
    voice(home, "loli", "refuse")

    with pytest.raises(RuntimeError, match="refused the line: the text does not fit"):
        say(tts)
    with pytest.raises(RuntimeError, match="refused"):
        say(tts)

    assert len(journal(home, "start")) == 1


def test_a_broken_audio_frame_kills_the_voice(tts: TtsSlot, home: Path) -> None:
    voice(home, "loli", "garbage")

    with pytest.raises(RuntimeError, match="bad_frame"):
        say(tts)


@pytest.mark.parametrize(
    ("mode", "named"),
    [
        ("fail_start", "failed to load the talker"),
        ("bad_rate", "'16000' Hz"),
        ("deaf", "no ready frame"),
    ],
)
def test_a_voice_that_never_gets_ready_fails_the_line(
    tts: TtsSlot, home: Path, monkeypatch: pytest.MonkeyPatch, mode: str, named: str
) -> None:
    voice(home, "loli", mode)
    monkeypatch.setattr(slot, "READY_S", 0.5)

    with pytest.raises(RuntimeError, match="not_ready") as failure:
        say(tts)

    assert named in str(failure.value)


def test_a_line_past_its_deadline_never_waits_for_a_voice(tts: TtsSlot, home: Path) -> None:
    voice(home, "loli", "deaf")
    CALL_GUARD.expires_at = time.monotonic() + 0.2

    with pytest.raises(CallExpired):
        say(tts)


def test_an_unknown_voice_or_method_is_bad_input(tts: TtsSlot, home: Path) -> None:
    with pytest.raises(BadInput):
        say(tts, "alice")
    with pytest.raises(BadInput):
        tts.invoke("sing", {}, {"voice": "loli", "text": TEXT})

    assert journal(home, "start") == []


def test_load_needs_every_file(home: Path) -> None:
    (home / "moriarty.gguf").unlink()

    with pytest.raises(RuntimeError, match=r"moriarty\.gguf"):
        TtsSlot().load(spec())


def test_load_needs_the_models_folder(home: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(TTS_HOME_ENV)

    with pytest.raises(RuntimeError, match=TTS_HOME_ENV):
        TtsSlot().load(spec())


def test_a_cuda_voice_runs_on_the_first_cuda_device_with_the_cuda_runtime_on_its_library_path(
    home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runtime, blas = cuda_wheels(tmp_path / "site", "cuda_runtime", "cublas")
    monkeypatch.syspath_prepend(str(tmp_path / "site"))
    tts = TtsSlot()
    tts.load(spec(TTS_DEVICES["cuda"]))
    try:
        spoken, _ = say(tts)
    finally:
        tts.unload()

    assert spoken == SPOKEN
    [start] = journal(home, "start")
    assert start["argv"] == [str(home / "loli.gguf"), str(home / "codec.gguf"), "CUDA0"]
    assert start["library_path"] == os.pathsep.join(map(str, [home / ENGINE, runtime, blas]))


def test_the_library_path_the_engine_inherits_comes_after_its_own_folders(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(process.LIBRARY_PATH, "/opt/drivers")
    tts = TtsSlot()
    tts.load(spec())
    try:
        say(tts)
    finally:
        tts.unload()

    [start] = journal(home, "start")
    assert start["argv"][-1] == "cpu"
    assert start["library_path"] == os.pathsep.join([str(home / ENGINE), "/opt/drivers"])


def test_a_device_the_speech_engine_does_not_know_is_refused_at_load(home: Path) -> None:
    with pytest.raises(RuntimeError, match="cannot run on device 'cuda'"):
        TtsSlot().load(spec("cuda"))


def test_cuda_libraries_take_the_first_folder_of_each_wheel_in_path_order(
    tmp_path: Path,
) -> None:
    early, late = tmp_path / "early", tmp_path / "late"
    [blas] = cuda_wheels(early, "cublas")
    runtime, _ = cuda_wheels(late, "cuda_runtime", "cublas")

    found = slot.cuda_libraries(["", str(tmp_path / "missing"), str(early), str(late)])

    assert found == (runtime, blas)
    assert slot.cuda_libraries([str(tmp_path / "missing")]) == ()


@pytest.mark.parametrize(
    ("chars", "tokens"),
    [(0, 49), (1, 52), (11, 80), (100, 333), (300, 901)],
)
def test_max_tokens_leaves_room_for_two_and_a_half_times_the_expected_audio(
    chars: int, tokens: int
) -> None:
    assert slot.max_tokens("я" * chars) == tokens
