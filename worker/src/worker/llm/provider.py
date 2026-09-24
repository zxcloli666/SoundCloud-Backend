from __future__ import annotations

from collections.abc import Mapping
from typing import Protocol

import aiohttp
import orjson

from worker.domain.deadline import Deadline

MAX_OUTPUT_TOKENS = 1024
ERROR_BODY_CHARS = 200


class ProviderError(Exception):
    pass


class Provider(Protocol):
    name: str

    async def complete(
        self, prompt: str, schema: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]: ...


async def post_json(
    session: aiohttp.ClientSession,
    url: str,
    headers: Mapping[str, str],
    body: Mapping[str, object],
    deadline: Deadline,
) -> Mapping[str, object]:
    timeout = aiohttp.ClientTimeout(total=deadline.remaining())
    try:
        async with session.post(
            url, data=orjson.dumps(body), headers=dict(headers), timeout=timeout
        ) as response:
            raw = await response.read()
            status = response.status
    except (aiohttp.ClientError, TimeoutError) as error:
        raise ProviderError(f"transport: {type(error).__name__}: {error}") from error
    if status >= 400:
        raise ProviderError(f"http {status}: {raw[:ERROR_BODY_CHARS].decode(errors='replace')}")
    return json_object(raw, "response body")


def json_object(raw: bytes | str, what: str) -> Mapping[str, object]:
    try:
        decoded = orjson.loads(raw)
    except orjson.JSONDecodeError as error:
        raise ProviderError(f"{what} is not JSON: {error}") from error
    if not isinstance(decoded, dict):
        raise ProviderError(f"{what} is {type(decoded).__name__}, not an object")
    return decoded
