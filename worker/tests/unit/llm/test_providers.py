from __future__ import annotations

import asyncio
import time
from collections.abc import AsyncIterator, Mapping
from dataclasses import dataclass, field

import aiohttp
import orjson
import pytest
from aiohttp import web
from aiohttp.test_utils import TestServer

from tests.fakes.engines import FakeEngines
from worker.domain.deadline import Deadline
from worker.domain.metadata.resolve import RESOLVE_SCHEMA
from worker.llm import anthropic, openai_compatible
from worker.llm.anthropic import AnthropicProvider
from worker.llm.local import LocalProvider
from worker.llm.openai_compatible import OpenAiCompatibleProvider
from worker.llm.provider import ProviderError
from worker.settings import ProviderSettings

SCHEMA: Mapping[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["name"],
    "properties": {"name": {"type": "string"}},
}


@dataclass
class FakeApi:
    status: int = 200
    body: object = None
    delay_s: float = 0.0
    requests: list[tuple[Mapping[str, str], dict[str, object]]] = field(default_factory=list)
    server: TestServer | None = None

    async def handle(self, request: web.Request) -> web.Response:
        self.requests.append((request.headers.copy(), await request.json()))
        if self.delay_s:
            await asyncio.sleep(self.delay_s)
        return web.Response(status=self.status, body=orjson.dumps(self.body))

    def url(self) -> str:
        assert self.server is not None
        return str(self.server.make_url("/v1/messages"))


@pytest.fixture
async def api() -> AsyncIterator[FakeApi]:
    fake = FakeApi()
    app = web.Application()
    app.router.add_post("/v1/messages", fake.handle)
    fake.server = TestServer(app)
    await fake.server.start_server()
    yield fake
    await fake.server.close()


def provider_settings(url: str, kind: str, model: str) -> ProviderSettings:
    return ProviderSettings(kind, url, model, "sk-secret", 4.0, 6.0)


def deadline(seconds: float = 5.0) -> Deadline:
    return Deadline(time.monotonic() + seconds)


def anthropic_reply(text: str, stop_reason: str = "end_turn") -> dict[str, object]:
    return {"content": [{"type": "text", "text": text}], "stop_reason": stop_reason}


def openai_reply(content: str, finish_reason: str = "stop") -> dict[str, object]:
    return {"choices": [{"message": {"content": content}, "finish_reason": finish_reason}]}


async def test_anthropic_sends_a_structured_output_request(api: FakeApi) -> None:
    api.body = anthropic_reply('{"name": "SZA"}')
    async with aiohttp.ClientSession() as session:
        provider = AnthropicProvider(
            "anthropic",
            provider_settings(api.url(), "anthropic", "claude-haiku-4-5-20251001"),
            session,
        )
        reply = await provider.complete("who?", SCHEMA, deadline())

    headers, body = api.requests[0]
    assert reply == {"name": "SZA"}
    assert headers["x-api-key"] == "sk-secret"
    assert headers["anthropic-version"] == "2023-06-01"
    assert body["model"] == "claude-haiku-4-5-20251001"
    assert body["messages"] == [{"role": "user", "content": "who?"}]
    assert body["output_config"] == {"format": {"type": "json_schema", "schema": dict(SCHEMA)}}


async def test_openai_compatible_sends_a_strict_json_schema(api: FakeApi) -> None:
    api.body = openai_reply('{"name": "SZA"}')
    async with aiohttp.ClientSession() as session:
        provider = OpenAiCompatibleProvider(
            "openai_compatible", provider_settings(api.url(), "openai_compatible", "m"), session
        )
        reply = await provider.complete("who?", SCHEMA, deadline())

    headers, body = api.requests[0]
    assert reply == {"name": "SZA"}
    assert headers["authorization"] == "Bearer sk-secret"
    assert body["response_format"] == {
        "type": "json_schema",
        "json_schema": {"name": "answer", "schema": dict(SCHEMA), "strict": True},
    }


@pytest.mark.parametrize("status", [400, 429, 500, 529])
async def test_http_errors_are_provider_errors(api: FakeApi, status: int) -> None:
    api.status = status
    api.body = {"error": {"type": "overloaded_error"}}
    async with aiohttp.ClientSession() as session:
        provider = AnthropicProvider(
            "anthropic", provider_settings(api.url(), "anthropic", "m"), session
        )
        with pytest.raises(ProviderError, match=f"http {status}"):
            await provider.complete("who?", SCHEMA, deadline())


async def test_a_slow_endpoint_is_cut_by_the_deadline(api: FakeApi) -> None:
    api.delay_s = 1.0
    api.body = anthropic_reply("{}")
    async with aiohttp.ClientSession() as session:
        provider = AnthropicProvider(
            "anthropic", provider_settings(api.url(), "anthropic", "m"), session
        )
        with pytest.raises(ProviderError, match="transport"):
            await provider.complete("who?", SCHEMA, deadline(0.2))


async def test_unreachable_endpoint_is_a_provider_error() -> None:
    async with aiohttp.ClientSession() as session:
        provider = AnthropicProvider(
            "anthropic",
            provider_settings("http://127.0.0.1:9/v1/messages", "anthropic", "m"),
            session,
        )
        with pytest.raises(ProviderError, match="transport"):
            await provider.complete("who?", SCHEMA, deadline())


@pytest.mark.parametrize(
    ("reply", "problem"),
    [
        (anthropic_reply('{"name": "SZA"}', "refusal"), "stop_reason=refusal"),
        (anthropic_reply('{"name": "S', "max_tokens"), "stop_reason=max_tokens"),
        (anthropic_reply("not json"), "not JSON"),
        (anthropic_reply("[1, 2]"), "not an object"),
        ({"content": [], "stop_reason": "end_turn"}, "no text block"),
    ],
)
def test_unusable_anthropic_replies(reply: Mapping[str, object], problem: str) -> None:
    with pytest.raises(ProviderError, match=problem):
        anthropic.answer(reply)


@pytest.mark.parametrize(
    ("reply", "problem"),
    [
        (openai_reply('{"name": "SZA"}', "length"), "finish_reason=length"),
        ({"choices": []}, "no choices"),
        ({"choices": [{"finish_reason": "stop", "message": {}}]}, "no message content"),
        (openai_reply("{"), "not JSON"),
    ],
)
def test_unusable_openai_replies(reply: Mapping[str, object], problem: str) -> None:
    with pytest.raises(ProviderError, match=problem):
        openai_compatible.answer(reply)


def test_request_schemas_follow_structured_output_rules() -> None:
    body = anthropic.request_body("m", "p", RESOLVE_SCHEMA)

    assert body["max_tokens"] == 1024
    assert "minimum" not in orjson.dumps(RESOLVE_SCHEMA).decode()
    assert '"additionalProperties":false' in orjson.dumps(RESOLVE_SCHEMA).decode()


async def test_local_provider_parses_the_generated_text() -> None:
    engines = FakeEngines()
    engines.generated = '{"name": "SZA"}'

    reply = await LocalProvider("local", engines).complete("who?", SCHEMA, deadline())

    assert reply == {"name": "SZA"}
    assert engines.calls[0][1]["max_new_tokens"] == 1024


async def test_local_provider_rejects_non_json_text() -> None:
    engines = FakeEngines()
    engines.generated = "I think it is SZA"

    with pytest.raises(ProviderError, match="not JSON"):
        await LocalProvider("local", engines).complete("who?", SCHEMA, deadline())
