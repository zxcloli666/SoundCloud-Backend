from __future__ import annotations

from collections.abc import Mapping

import aiohttp

from worker.domain.deadline import Deadline
from worker.llm.provider import MAX_OUTPUT_TOKENS, ProviderError, json_object, post_json
from worker.settings import ProviderSettings

API_VERSION = "2023-06-01"


class AnthropicProvider:
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
                "x-api-key": self.settings.api_key,
                "anthropic-version": API_VERSION,
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
        "output_config": {"format": {"type": "json_schema", "schema": schema}},
    }


def answer(reply: Mapping[str, object]) -> Mapping[str, object]:
    stop_reason = reply.get("stop_reason")
    if stop_reason != "end_turn":
        raise ProviderError(f"stop_reason={stop_reason}")
    content = reply.get("content")
    blocks = content if isinstance(content, list) else []
    texts = [block["text"] for block in blocks if is_text_block(block)]
    if not texts:
        raise ProviderError("reply carries no text block")
    return json_object("".join(texts), "answer text")


def is_text_block(block: object) -> bool:
    return (
        isinstance(block, dict)
        and block.get("type") == "text"
        and isinstance(block.get("text"), str)
    )
