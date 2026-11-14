from __future__ import annotations

import base64
import logging
from collections.abc import Collection, Mapping
from dataclasses import dataclass

from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.metadata import fields
from worker.domain.outcome import Failure, PermanentFailure, Reason, TransientFailure
from worker.domain.ports import ClipEncoder, Engines, EngineUnavailable, Float32Array, Speech
from worker.observability.counters import Counters

SCHEMA = "speak.rpc.say"
REPLY_SCHEMA = "speak.rpc.say.reply"
SAMPLE_RATE = 24_000
ENCODE_MARGIN_S = 3.0

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Line:
    voice: str
    text: str


class Speaker:
    def __init__(
        self,
        engines: Engines,
        encoder: ClipEncoder,
        contract: Contract,
        voices: Collection[str],
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._encoder = encoder
        self._contract = contract
        self._voices = voices
        self._counters = counters

    async def say(self, request: Mapping[str, object], deadline: Deadline) -> Mapping[str, object]:
        line = self._line(request)
        started = deadline.now()
        speech = await self._speak(line, deadline.minus(ENCODE_MARGIN_S))
        if not speech.samples.size:
            self._failed(line, "no_audio", "the engine returned no audio")
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, "the engine returned no audio")
        clip = await self._encode(line, speech.samples, deadline)
        seconds = round(speech.samples.size / SAMPLE_RATE, 3)
        data = {"seconds": seconds, "clip": base64.b64encode(clip).decode("ascii")}
        self._check(line, data)
        self._said(line, speech, seconds, len(clip), deadline.now() - started)
        return data

    def _line(self, request: Mapping[str, object]) -> Line:
        problems = self._contract.validate(SCHEMA, request)
        if problems:
            paths = sorted({problem.partition(":")[0] for problem in problems})
            raise self._rejected("schema", paths=paths)
        line = Line(fields.required_text(request, "voice"), fields.required_text(request, "text"))
        if line.voice not in self._voices:
            raise self._rejected("voice", voice_chars=len(line.voice))
        if len(line.text) > self._contract.say_limits.text_chars:
            raise self._rejected("text_chars", text_chars=len(line.text))
        return line

    async def _speak(self, line: Line, deadline: Deadline) -> Speech:
        try:
            return await self._engines.say(line.voice, line.text, deadline)
        except EngineUnavailable as error:
            self._failed(line, "unavailable", str(error))
            raise TransientFailure(Reason.INTERNAL_ERROR, str(error)) from error
        except Failure as failure:
            self._failed(line, failure.reason.value, str(failure))
            raise

    async def _encode(self, line: Line, samples: Float32Array, deadline: Deadline) -> bytes:
        try:
            return await self._encoder.encode(samples, deadline)
        except Failure as failure:
            self._failed(line, "encode", str(failure))
            raise TransientFailure(Reason.INTERNAL_ERROR, str(failure)) from failure

    def _check(self, line: Line, data: Mapping[str, object]) -> None:
        problems = self._contract.validate(REPLY_SCHEMA, {"ok": True, "data": data})
        if not problems:
            return
        paths = sorted({problem.partition(":")[0] for problem in problems})
        detail = f"the reply breaks its schema at {paths}, clip of {len(str(data['clip']))} chars"
        self._failed(line, "reply", detail)
        raise TransientFailure(Reason.INTERNAL_ERROR, detail)

    def _said(
        self, line: Line, speech: Speech, seconds: float, clip_bytes: int, wall_s: float
    ) -> None:
        self._counters.inc("speak_said_total", voice=line.voice)
        self._counters.observe("speak_wall_ms", wall_s * 1000.0, voice=line.voice)
        if speech.loaded:
            self._counters.inc("speak_voice_loads_total", voice=line.voice)
        log.info(
            "speak_said",
            extra={
                "voice": line.voice,
                "text_chars": len(line.text),
                "audio_s": seconds,
                "clip_bytes": clip_bytes,
                "wall_s": round(wall_s, 3),
                "load_s": round(speech.load_s, 3),
            },
        )

    def _failed(self, line: Line, why: str, detail: str) -> None:
        self._counters.inc("speak_failed_total", why=why)
        log.warning(
            "speak_failed",
            extra={
                "voice": line.voice,
                "why": why,
                "detail": detail,
                "text_chars": len(line.text),
            },
        )

    def _rejected(self, why: str, **detail: object) -> PermanentFailure:
        self._counters.inc("speak_rejected_total", why=why)
        log.warning("speak_rejected", extra={"why": why, **detail})
        return PermanentFailure(Reason.INVALID_REQUEST, why)
