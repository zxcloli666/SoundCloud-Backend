from __future__ import annotations

from collections.abc import Mapping
from typing import Protocol

from worker.domain.deadline import Deadline
from worker.llm.provider import MAX_OUTPUT_TOKENS, json_object


class Generator(Protocol):
    async def generate(
        self,
        prompt: str,
        schema: Mapping[str, object],
        max_new_tokens: int,
        deadline: Deadline,
    ) -> str: ...


class LocalProvider:
    def __init__(self, name: str, engines: Generator) -> None:
        self.name = name
        self.engines = engines

    async def complete(
        self, prompt: str, schema: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        text = await self.engines.generate(prompt, schema, MAX_OUTPUT_TOKENS, deadline)
        return json_object(text, "local answer")
