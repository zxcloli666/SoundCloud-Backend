from __future__ import annotations

from collections.abc import Mapping

import aiohttp

from worker.domain.deadline import Deadline
from worker.llm.provider import MAX_OUTPUT_TOKENS, ProviderError, json_object, post_json
from worker.settings import ProviderSettings

SCHEMA_NAME = "answer"


class OpenAiCompatibleProvider:
    def __init__(
        self, name: str, settings: ProviderSettings, session: aiohttp.ClientSession
    ) -> None:
        self.name = name
        self.settings = settings
        self.session = session

    async def complete(
        self, prompt: str, schema: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        reply = await post_json(
            self.session,
            self.settings.endpoint,
            {
                "authorization": f"Bearer {self.settings.api_key}",
                "content-type": "application/json",
            },
            request_body(self.settings.model, prompt, schema),
            deadline,
        )
        return answer(reply)


def request_body(model: str, prompt: str, schema: Mapping[str, object]) -> dict[str, object]:
    return {
        "model": model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "messages": [{"role": "user", "content": prompt}],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": SCHEMA_NAME, "schema": schema, "strict": True},
        },
    }


def answer(reply: Mapping[str, object]) -> Mapping[str, object]:
    choices = reply.get("choices")
    first = choices[0] if isinstance(choices, list) and choices else None
    if not isinstance(first, dict):
        raise ProviderError("reply carries no choices")
    if first.get("finish_reason") != "stop":
        raise ProviderError(f"finish_reason={first.get('finish_reason')}")
    message = first.get("message")
    content = message.get("content") if isinstance(message, dict) else None
    if not isinstance(content, str):
        raise ProviderError("reply carries no message content")
    return json_object(content, "answer text")
