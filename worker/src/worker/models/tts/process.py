from __future__ import annotations

import os
import queue
import subprocess
import threading
import time
from collections import deque
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import IO

import numpy as np
from numpy.typing import NDArray

from worker.models.tts import wire
from worker.observability.logging import JsonLog
from worker.settings import TTS_SLOT

RATE = 24_000
DEVICE = "cpu"
STALL_S = 20.0
CANCEL_S = 5.0
EXIT_S = 2.0
ERROR_LINES = 20
DETAIL_CHARS = 300
LIBRARY_PATH = "LD_LIBRARY_PATH"

Samples = NDArray[np.float32]

log = JsonLog(component=TTS_SLOT)


class EngineLost(Exception):
    def __init__(self, why: str, detail: str) -> None:
        super().__init__(f"{why}: {detail}")
        self.why = why
        self.detail = detail


class EngineRefused(Exception):
    pass


class SayCancelled(Exception):
    pass


@dataclass(frozen=True)
class Launch:
    program: Path
    talker: Path
    codec: Path

    def argv(self) -> list[str]:
        return [str(self.program), str(self.talker), str(self.codec), DEVICE]

    def env(self, environ: Mapping[str, str]) -> dict[str, str]:
        return {**environ, LIBRARY_PATH: str(self.program.parent)}


@dataclass(frozen=True)
class Line:
    seed: int
    max_tokens: int
    speaker: str
    lang: str
    text: str

    def payload(self) -> bytes:
        fields = (str(self.seed), str(self.max_tokens), self.speaker, self.lang, self.text)
        return "\n".join(fields).encode()


@dataclass(frozen=True)
class Ended:
    detail: str


class VoiceProcess:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self._process = process
        self._input = piped(process.stdin)
        self._frames: queue.Queue[wire.Frame | Ended] = queue.Queue()
        self._errors: deque[str] = deque(maxlen=ERROR_LINES)
        self._pumps = (
            started(pump_frames, piped(process.stdout), self._frames),
            started(pump_lines, piped(process.stderr), self._errors),
        )
        self._last = 0

    @classmethod
    def start(cls, launch: Launch, until: float) -> VoiceProcess:
        try:
            process = subprocess.Popen(
                launch.argv(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=launch.program.parent,
                env=launch.env(os.environ),
            )
        except OSError as error:
            raise EngineLost("not_started", str(error)) from error
        voice = cls(process)
        try:
            voice._ready(until)
        except EngineLost as lost:
            voice.kill()
            raise EngineLost("not_ready", voice._last_words(lost.detail)) from lost
        return voice

    @property
    def pid(self) -> int:
        return self._process.pid

    @property
    def alive(self) -> bool:
        return self._process.poll() is None

    def say(self, line: Line, expires_at: float) -> Samples:
        self._last += 1
        number = self._last
        self._send(wire.Frame(wire.SAY, number, line.payload()))
        chunks: list[Samples] = []
        give_up_at = time.monotonic() + STALL_S
        cancelled = False
        while True:
            if not cancelled and time.monotonic() >= expires_at:
                self._send(wire.Frame(wire.CANCEL, number))
                cancelled = True
                give_up_at = time.monotonic() + CANCEL_S
            frame = self._next(give_up_at if cancelled else min(give_up_at, expires_at))
            if frame is None:
                if time.monotonic() >= give_up_at:
                    self.kill()
                    raise EngineLost("stalled", f"no frame in time, cancelled={cancelled}")
                continue
            if frame.id != number:
                continue
            if not cancelled:
                give_up_at = time.monotonic() + STALL_S
            if frame.kind == wire.AUDIO and not cancelled:
                chunks.append(self._samples(frame))
            elif frame.kind == wire.DONE:
                if cancelled:
                    raise SayCancelled(f"cancelled after {len(chunks)} audio frames")
                return np.concatenate(chunks) if chunks else np.zeros(0, np.float32)
            elif frame.kind == wire.FAILED:
                raise EngineRefused(wire.text(frame.payload, DETAIL_CHARS))

    def stop(self) -> int:
        self._close_input()
        return self._reap()

    def kill(self) -> None:
        self._process.kill()
        self._process.wait()
        self._close_input()
        self._join()

    def _ready(self, until: float) -> None:
        frame = self._next(until)
        if frame is None:
            raise EngineLost("not_ready", "no ready frame in time")
        greeting = wire.text(frame.payload, DETAIL_CHARS)
        if frame.kind == wire.FAILED:
            raise EngineLost("not_ready", greeting)
        rate = greeting.partition("\n")[0]
        if frame.kind != wire.READY or rate != str(RATE):
            raise EngineLost("not_ready", f"greeted with {chr(frame.kind)!r} at {rate!r} Hz")

    def _next(self, until: float) -> wire.Frame | None:
        try:
            item = self._frames.get(timeout=max(0.0, until - time.monotonic()))
        except queue.Empty:
            return None
        if isinstance(item, Ended):
            code = self._reap()
            raise EngineLost("died", f"{item.detail}, exit code {code}")
        return item

    def _samples(self, frame: wire.Frame) -> Samples:
        try:
            return wire.samples(frame.payload)
        except wire.BadFrame as error:
            self.kill()
            raise EngineLost("bad_frame", str(error)) from error

    def _send(self, frame: wire.Frame) -> None:
        try:
            self._input.write(wire.encode(frame))
            self._input.flush()
        except OSError as error:
            code = self._reap()
            raise EngineLost("died", f"input closed: {error}, exit code {code}") from error

    def _reap(self) -> int:
        try:
            code = self._process.wait(EXIT_S)
        except subprocess.TimeoutExpired:
            self._process.kill()
            code = self._process.wait()
        self._close_input()
        self._join()
        return code

    def _close_input(self) -> None:
        try:
            self._input.close()
        except OSError as error:
            log.warning("tts_input_close_failed", pid=self.pid, error=str(error))

    def _join(self) -> None:
        for pump in self._pumps:
            pump.join(EXIT_S)

    def _last_words(self, detail: str) -> str:
        return "; ".join((detail, *self._errors))[: DETAIL_CHARS * 4]


def piped(stream: IO[bytes] | None) -> IO[bytes]:
    if stream is None:
        raise RuntimeError("the speech engine was started without a pipe")
    return stream


def started[T](
    target: Callable[[IO[bytes], T], None], stream: IO[bytes], into: T
) -> threading.Thread:
    thread = threading.Thread(target=target, args=(stream, into), daemon=True)
    thread.start()
    return thread


def pump_frames(stream: IO[bytes], frames: queue.Queue[wire.Frame | Ended]) -> None:
    with stream:
        frames.put(Ended(read_frames(stream, frames)))


def read_frames(stream: IO[bytes], frames: queue.Queue[wire.Frame | Ended]) -> str:
    try:
        while (frame := wire.read(stream)) is not None:
            frames.put(frame)
    except (wire.BadFrame, OSError) as error:
        return f"unreadable output: {error}"
    return "the output closed"


def pump_lines(stream: IO[bytes], lines: deque[str]) -> None:
    with stream:
        try:
            for raw in stream:
                lines.append(raw.decode(errors="replace").rstrip())
        except OSError as error:
            lines.append(f"unreadable error output: {error}")
