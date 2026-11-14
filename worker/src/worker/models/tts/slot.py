from __future__ import annotations

import os
import sys
import time
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

from worker.fetch_models import TTS_HOME_ENV
from worker.models.tts.process import (
    EngineLost,
    EngineRefused,
    Launch,
    Line,
    SayCancelled,
    VoiceProcess,
)
from worker.observability.logging import JsonLog
from worker.runtime.protocol import CALL_GUARD, Arrays, BadInput, CallExpired, SlotSpec
from worker.settings import TTS_DEVICES, TTS_METHOD, TTS_SLOT

SEED = 1
LANG = "auto"
LEAD_S = 0.3
CHARS_PER_S = 11.0
FRAMES_PER_S = 12.5
TOKEN_ROOM = 2.5
TOKEN_SPARE = 40
READY_S = 60.0
AUDIO = "audio"
ENGINE_DEVICES: Mapping[str, str] = {TTS_DEVICES["cpu"]: "cpu", TTS_DEVICES["cuda"]: "CUDA0"}
CUDA_RUNTIME_PACKAGES = ("cuda_runtime", "cublas")

log = JsonLog(component=TTS_SLOT)


@dataclass(frozen=True)
class Voice:
    talker: Path
    speaker: str


@dataclass(frozen=True)
class Options:
    program: Path
    codec: Path
    voices: Mapping[str, Voice]
    device: str
    libraries: tuple[Path, ...]

    @classmethod
    def of(cls, spec: SlotSpec, home: Path) -> Options:
        program, codec = spec.options.get("program"), spec.options.get("codec")
        voices = spec.options.get("voices")
        if not (
            isinstance(program, str) and isinstance(codec, str) and isinstance(voices, Mapping)
        ):
            raise RuntimeError(f"{TTS_SLOT} needs the options program, codec and voices")
        device = ENGINE_DEVICES.get(spec.device)
        if device is None:
            raise RuntimeError(f"{TTS_SLOT} cannot run on device {spec.device!r}")
        return cls(
            home / program,
            home / codec,
            {str(name): voice_option(home, name, voice) for name, voice in voices.items()},
            device,
            cuda_libraries(sys.path) if spec.device == TTS_DEVICES["cuda"] else (),
        )

    def files(self) -> list[Path]:
        return [self.program, self.codec, *(voice.talker for voice in self.voices.values())]

    def launch(self, voice: Voice) -> Launch:
        return Launch(self.program, voice.talker, self.codec, self.device, self.libraries)


@dataclass(frozen=True)
class Running:
    voice: str
    process: VoiceProcess


class TtsSlot:
    def __init__(self) -> None:
        self._options: Options | None = None
        self._running: Running | None = None

    def load(self, spec: SlotSpec) -> None:
        options = Options.of(spec, models_home(os.environ))
        missing = [str(path) for path in options.files() if not path.is_file()]
        if missing:
            raise RuntimeError(f"{TTS_SLOT} files are missing: {missing}")
        self._options = options

    def warmup(self) -> None:
        return None

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != TTS_METHOD:
            raise BadInput(f"{TTS_SLOT} has no method {method!r}")
        voice, text = args.get("voice"), args.get("text")
        if not isinstance(voice, str) or not isinstance(text, str):
            raise TypeError(f"{TTS_METHOD} got arguments of the wrong types")
        return self.say(voice, text)

    def unload(self) -> None:
        self._stop("unload")

    def say(self, name: str, text: str) -> tuple[Arrays, Mapping[str, object]]:
        if self._options is None:
            raise RuntimeError(f"{TTS_SLOT} is not loaded")
        voice = self._options.voices.get(name)
        if voice is None:
            raise BadInput(f"{TTS_SLOT} has no voice {name!r}")
        process, load_s = self._process_for(name, voice, self._options)
        CALL_GUARD.check()
        line = Line(SEED, max_tokens(text), voice.speaker, LANG, text)
        try:
            audio = process.say(line, CALL_GUARD.expires_at)
        except SayCancelled as cancelled:
            log.info("tts_say_cancelled", voice=name, text_chars=len(text))
            raise CallExpired("the deadline cancelled the speech") from cancelled
        except EngineLost as lost:
            self._running = None
            log.error("tts_voice_lost", voice=name, why=lost.why, detail=lost.detail)
            raise failure_of(lost) from lost
        except EngineRefused as refused:
            log.warning("tts_say_refused", voice=name, detail=str(refused))
            raise RuntimeError(f"the speech engine refused the line: {refused}") from refused
        loaded = load_s is not None
        return {AUDIO: audio}, {"loaded": loaded, "load_s": round(load_s or 0.0, 3)}

    def _process_for(
        self, name: str, voice: Voice, options: Options
    ) -> tuple[VoiceProcess, float | None]:
        running = self._running
        if running is not None and running.voice == name and running.process.alive:
            return running.process, None
        if running is not None:
            self._stop("switch" if running.voice != name else "died")
        started = time.monotonic()
        try:
            process = VoiceProcess.start(
                options.launch(voice), min(started + READY_S, CALL_GUARD.expires_at)
            )
        except EngineLost as lost:
            log.error("tts_voice_load_failed", voice=name, why=lost.why, detail=lost.detail)
            raise failure_of(lost) from lost
        self._running = Running(name, process)
        load_s = time.monotonic() - started
        log.info("tts_voice_loaded", voice=name, pid=process.pid, seconds=round(load_s, 2))
        return process, load_s

    def _stop(self, why: str) -> None:
        running = self._running
        if running is None:
            return
        self._running = None
        code = running.process.stop()
        log.info("tts_voice_unloaded", voice=running.voice, why=why, exit_code=code)


def max_tokens(text: str) -> int:
    seconds = LEAD_S + len(text) / CHARS_PER_S
    return int(seconds * FRAMES_PER_S * TOKEN_ROOM) + TOKEN_SPARE


def failure_of(lost: EngineLost) -> Exception:
    if time.monotonic() >= CALL_GUARD.expires_at:
        return CallExpired(f"the speech engine was {lost.why} at the deadline")
    return RuntimeError(f"the speech engine {lost.why}: {lost.detail}")


def cuda_libraries(paths: Iterable[str]) -> tuple[Path, ...]:
    places = [Path(entry) / "nvidia" for entry in paths if entry]
    found: list[Path] = []
    for package in CUDA_RUNTIME_PACKAGES:
        folders = [place / package / "lib" for place in places]
        found += [folder for folder in folders if folder.is_dir()][:1]
    return tuple(found)


def models_home(environ: Mapping[str, str]) -> Path:
    home = environ.get(TTS_HOME_ENV, "")
    if not home:
        raise RuntimeError(f"{TTS_HOME_ENV} is not set")
    return Path(home)


def voice_option(home: Path, name: object, raw: object) -> Voice:
    talker = raw.get("talker") if isinstance(raw, Mapping) else None
    speaker = raw.get("speaker") if isinstance(raw, Mapping) else None
    if not isinstance(talker, str) or not isinstance(speaker, str):
        raise RuntimeError(f"{TTS_SLOT} voice {name} needs the text options talker and speaker")
    return Voice(home / talker, speaker)
