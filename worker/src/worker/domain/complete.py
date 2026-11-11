from __future__ import annotations

import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass

import orjson

from worker.contract import CompleteLimits
from worker.domain.deadline import Deadline
from worker.domain.metadata import fields
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.domain.ports import ChatMessage, ChatRole, Completion, Engines, EngineUnavailable
from worker.observability.counters import Counters
from worker.settings import LocalLlmSettings

METHOD = "complete"
RPC_MARGIN_S = 0.5
MAX_IN_FLIGHT = 4
ROLES: Mapping[str, ChatRole] = {"user": "user", "assistant": "assistant"}

log = logging.getLogger(__name__)


class Completer:
    def __init__(
        self,
        engines: Engines,
        limits: CompleteLimits,
        local: LocalLlmSettings,
        model: str,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._limits = limits
        self._local = local
        self._model = model
        self._counters = counters
        self._in_flight = 0

    async def complete(
        self, request: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        if not self._local.enabled:
            self._counters.inc("complete_unavailable_total", state="off")
            raise TransientFailure(Reason.INTERNAL_ERROR, "the local model is off on this worker")
        deadline.minus(RPC_MARGIN_S).check(METHOD)
        prompt = Prompt.parse(request, self._limits)
        if self._in_flight >= MAX_IN_FLIGHT:
            self._counters.inc("complete_rejected_total", why="busy")
            log.warning("complete_model_busy", extra={"in_flight": self._in_flight})
            raise TransientFailure(Reason.DEADLINE_EXCEEDED, "the local model is busy")
        self._in_flight += 1
        try:
            completion = await self._ask(prompt, deadline)
        finally:
            self._in_flight -= 1
        self._check(completion)
        self._counters.inc("complete_answers_total")
        self._counters.inc("complete_input_tokens_total", by=completion.input_tokens)
        self._counters.inc("complete_reused_tokens_total", by=completion.reused_tokens)
        self._counters.inc("complete_output_tokens_total", by=completion.output_tokens)
        log.info(
            "complete_answered",
            extra={
                "input_tokens": completion.input_tokens,
                "reused_tokens": completion.reused_tokens,
                "output_tokens": completion.output_tokens,
            },
        )
        return {
            "text": completion.text,
            "model": self._model,
            "input_tokens": completion.input_tokens,
            "output_tokens": completion.output_tokens,
        }

    async def _ask(self, prompt: Prompt, deadline: Deadline) -> Completion:
        try:
            return await self._engines.complete(
                prompt.system, prompt.messages, prompt.schema, prompt.max_tokens, deadline
            )
        except EngineUnavailable as error:
            self._counters.inc("complete_unavailable_total", state=error.state)
            log.warning("complete_model_unavailable", extra={"state": error.state})
            raise TransientFailure(Reason.INTERNAL_ERROR, str(error)) from error

    def _check(self, completion: Completion) -> None:
        if not completion.finished:
            self._counters.inc("complete_rejected_total", why="cut_at_max_tokens")
            raise PermanentFailure(
                Reason.MODEL_OUTPUT_INVALID,
                f"the answer did not end within {completion.output_tokens} tokens",
            )
        if not is_json_object(completion.text):
            self._counters.inc("complete_rejected_total", why="not_a_json_object")
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, "the answer is not a JSON object")


@dataclass(frozen=True)
class Prompt:
    system: str
    messages: Sequence[ChatMessage]
    schema: Mapping[str, object]
    max_tokens: int

    @classmethod
    def parse(cls, request: Mapping[str, object], limits: CompleteLimits) -> Prompt:
        system = fields.required_text(request, "system")
        if utf8_bytes(system) > limits.system_bytes:
            raise invalid(f"system is longer than {limits.system_bytes} bytes")
        schema = fields.required_mapping(request, "schema")
        if len(orjson.dumps(schema)) > limits.schema_bytes:
            raise invalid(f"schema is longer than {limits.schema_bytes} bytes")
        max_tokens = fields.required_int(request, "max_tokens")
        if not 1 <= max_tokens <= limits.max_tokens:
            raise invalid(f"max_tokens must be within 1..{limits.max_tokens}")
        return cls(system, parse_messages(request, limits), schema, max_tokens)


def parse_messages(request: Mapping[str, object], limits: CompleteLimits) -> list[ChatMessage]:
    raw_messages = fields.required_list(request, "messages")
    if not 1 <= len(raw_messages) <= limits.messages:
        raise invalid(f"messages must hold 1..{limits.messages} items")
    return [parse_message(raw, limits) for raw in raw_messages]


def parse_message(raw: object, limits: CompleteLimits) -> ChatMessage:
    if not isinstance(raw, Mapping):
        raise invalid("a message must be an object")
    role = ROLES.get(fields.required_text(raw, "role"))
    if role is None:
        raise invalid(f"a message role must be one of {sorted(ROLES)}")
    content = fields.required_text(raw, "content")
    if not content or utf8_bytes(content) > limits.message_bytes:
        raise invalid(f"a message content must hold 1..{limits.message_bytes} bytes")
    return ChatMessage(role, content)


def utf8_bytes(text: str) -> int:
    return len(text.encode("utf-8", errors="surrogatepass"))


def is_json_object(text: str) -> bool:
    try:
        return isinstance(orjson.loads(text), dict)
    except orjson.JSONDecodeError:
        return False


def invalid(detail: str) -> PermanentFailure:
    return PermanentFailure(Reason.INVALID_REQUEST, detail)
