from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from typing import Protocol

from worker.domain.deadline import Deadline
from worker.domain.ports import ChatMessage, Completion
from worker.llm.provider import MAX_OUTPUT_TOKENS, json_object

SYSTEM_PROMPT = (
    "You answer with exactly one JSON object and nothing else. "
    "The object must validate against this JSON Schema: "
)


class LocalModel(Protocol):
    async def complete(
        self,
        system: str,
        messages: Sequence[ChatMessage],
        schema: Mapping[str, object],
        max_tokens: int,
        deadline: Deadline,
    ) -> Completion: ...


class LocalProvider:
    def __init__(self, name: str, engines: LocalModel) -> None:
        self.name = name
        self.engines = engines

    async def complete(
        self, prompt: str, schema: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        completion = await self.engines.complete(
            SYSTEM_PROMPT + json.dumps(schema),
            [ChatMessage("user", prompt)],
            schema,
            MAX_OUTPUT_TOKENS,
            deadline,
        )
        return json_object(completion.text, "local answer")
