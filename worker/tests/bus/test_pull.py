from __future__ import annotations

import asyncio
import json

import nats.errors
import nats.js.errors
import pytest
from nats.aio.msg import Msg

from tests.fakes.clock import FakeClock
from worker.bus.pull import NANOS_PER_S, pulled

EXPIRES_S = 5.0


class OpenRequest:
    def __init__(self) -> None:
        self._nc = self
        self._sub = self
        self._nms = "$JS.API.CONSUMER.MSG.NEXT.STREAM.durable"
        self._deliver = "_INBOX.test"
        self.inbox: asyncio.Queue[Msg] = asyncio.Queue()
        self.requests: list[tuple[str, dict[str, object], str]] = []
        self.reads = 0

    async def publish(self, subject: str, payload: bytes, reply: str) -> None:
        self.requests.append((subject, json.loads(payload), reply))

    async def next_msg(self, timeout: float) -> Msg:
        self.reads += 1
        try:
            return await asyncio.wait_for(self.inbox.get(), 0.05)
        except TimeoutError as error:
            raise nats.errors.TimeoutError from error

    def deliver(self, body: bytes) -> None:
        self.inbox.put_nowait(Msg(None, subject="lane.task", data=body))

    def status(self, code: str) -> None:
        headers = {"Status": code, "Description": "scripted"}
        self.inbox.put_nowait(Msg(None, subject=self._deliver, headers=headers))


async def test_a_pull_is_one_shot_without_heartbeats() -> None:
    request = OpenRequest()
    request.status("408")

    taken = [msg async for msg in pulled(request, 16, EXPIRES_S, FakeClock())]

    assert taken == []
    assert request.requests == [
        (
            request._nms,
            {"batch": 16, "expires": int(EXPIRES_S * NANOS_PER_S), "no_wait": True},
            request._deliver,
        )
    ]


async def test_a_message_is_handed_over_before_the_request_closes() -> None:
    request = OpenRequest()
    request.deliver(b"first")
    messages = pulled(request, 16, EXPIRES_S, FakeClock())

    first = await anext(messages)
    await messages.aclose()

    assert first.data == b"first"
    assert request.reads == 1


async def test_a_full_batch_ends_the_pull_without_another_read() -> None:
    request = OpenRequest()
    for body in (b"a", b"b", b"c"):
        request.deliver(body)

    taken = [msg.data async for msg in pulled(request, 2, EXPIRES_S, FakeClock())]

    assert taken == [b"a", b"b"]
    assert request.reads == 2


@pytest.mark.parametrize("code", ["404", "408", "409"])
async def test_a_closing_status_ends_the_pull_after_what_was_delivered(code: str) -> None:
    request = OpenRequest()
    request.deliver(b"a")
    request.status(code)
    request.deliver(b"next request")

    taken = [msg.data async for msg in pulled(request, 16, EXPIRES_S, FakeClock())]

    assert taken == [b"a"]


async def test_any_other_status_is_an_error() -> None:
    request = OpenRequest()
    request.status("400")

    with pytest.raises(nats.js.errors.APIError):
        [msg async for msg in pulled(request, 16, EXPIRES_S, FakeClock())]


async def test_no_reply_before_the_expiry_ends_the_pull() -> None:
    request = OpenRequest()

    taken = [msg async for msg in pulled(request, 16, EXPIRES_S, FakeClock())]

    assert taken == []
    assert request.reads == 1
