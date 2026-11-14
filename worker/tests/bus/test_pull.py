from __future__ import annotations

import asyncio
import json

import nats.errors
import nats.js.errors
import pytest
from nats.aio.client import Client
from nats.aio.msg import Msg
from nats.aio.subscription import Subscription
from nats.js import JetStreamContext

from tests.fakes.clock import FakeClock
from worker.bus.pull import NANOS_PER_S, REPLY_SLACK_S, PullUnanswered, pulled
from worker.domain.deadline import MonotonicClock

EXPIRES_S = 5.0
LAST_MOMENT_S = 0.25
NEXT_SUBJECT = "$JS.API.CONSUMER.MSG.NEXT.STREAM.durable"
INBOX = "_INBOX.worker.1"


class OpenRequest:
    def __init__(self, clock: FakeClock) -> None:
        self._nc = self
        self._sub = self
        self._nms = NEXT_SUBJECT
        self._deliver = INBOX
        self.clock = clock
        self.inbox: asyncio.Queue[Msg] = asyncio.Queue()
        self.requests: list[tuple[str, dict[str, object], str]] = []
        self.reads = 0

    async def publish(self, subject: str, payload: bytes, reply: str) -> None:
        self.requests.append((subject, json.loads(payload), reply))

    async def next_msg(self, timeout: float) -> Msg:
        self.reads += 1
        arrival = asyncio.ensure_future(self.inbox.get())
        expiry = asyncio.ensure_future(self.clock.sleep(timeout))
        await asyncio.wait({arrival, expiry}, return_when=asyncio.FIRST_COMPLETED)
        expiry.cancel()
        if arrival.done():
            return arrival.result()
        arrival.cancel()
        raise nats.errors.TimeoutError

    def deliver(self, body: bytes) -> None:
        self.inbox.put_nowait(Msg(None, subject="lane.task", data=body))

    def status(self, code: str, description: str | None = "scripted") -> None:
        headers = {"Status": code}
        if description is not None:
            headers["Description"] = description
        self.inbox.put_nowait(Msg(None, subject=self._deliver, headers=headers))


async def test_a_pull_waits_on_the_server_without_no_wait_or_heartbeats() -> None:
    request = OpenRequest(FakeClock())
    request.status("408")

    taken = [msg async for msg in pulled(request, 16, EXPIRES_S, request.clock)]

    assert taken == []
    assert request.requests == [
        (NEXT_SUBJECT, {"batch": 16, "expires": int(EXPIRES_S * NANOS_PER_S)}, INBOX)
    ]


async def test_a_pull_carries_the_priority_group_and_priority_it_is_given() -> None:
    request = OpenRequest(FakeClock())
    request.status("408")
    priority = {"group": "speak", "priority": 1}

    taken = [msg async for msg in pulled(request, 8, EXPIRES_S, request.clock, priority)]

    assert taken == []
    expires = int(EXPIRES_S * NANOS_PER_S)
    assert request.requests == [
        (NEXT_SUBJECT, {"batch": 8, "expires": expires, "group": "speak", "priority": 1}, INBOX)
    ]


async def test_a_message_is_handed_over_before_the_request_closes() -> None:
    request = OpenRequest(FakeClock())
    request.deliver(b"first")
    messages = pulled(request, 16, EXPIRES_S, request.clock)

    first = await anext(messages)
    await messages.aclose()

    assert first.data == b"first"
    assert request.reads == 1


async def test_a_full_batch_ends_the_pull_without_another_read() -> None:
    request = OpenRequest(FakeClock())
    for body in (b"a", b"b", b"c"):
        request.deliver(body)

    taken = [msg.data async for msg in pulled(request, 2, EXPIRES_S, request.clock)]

    assert taken == [b"a", b"b"]
    assert request.reads == 2


async def test_the_expiry_status_ends_the_pull_after_what_was_delivered() -> None:
    request = OpenRequest(FakeClock())
    request.deliver(b"a")
    request.status("408")
    request.deliver(b"next request")

    taken = [msg.data async for msg in pulled(request, 16, EXPIRES_S, request.clock)]

    assert taken == [b"a"]


@pytest.mark.parametrize(
    ("code", "description"),
    [("409", "Exceeded MaxWaiting"), ("409", None), ("404", "No Messages"), ("400", "Bad Request")],
)
async def test_any_other_status_is_an_error_after_what_was_delivered(
    code: str, description: str | None
) -> None:
    request = OpenRequest(FakeClock())
    request.deliver(b"a")
    request.status(code, description)
    taken = []

    with pytest.raises(nats.js.errors.APIError) as refused:
        async for msg in pulled(request, 16, EXPIRES_S, request.clock):
            taken.append(msg.data)

    assert taken == [b"a"]
    assert (refused.value.code, refused.value.description) == (int(code), description)


async def test_a_pull_is_unanswered_only_after_its_expiry_and_the_reply_slack() -> None:
    clock = FakeClock()
    request = OpenRequest(clock)

    async def collect() -> list[Msg]:
        return [msg async for msg in pulled(request, 16, EXPIRES_S, clock)]

    waiting = asyncio.ensure_future(collect())
    await clock.tick()
    await clock.tick(EXPIRES_S + REPLY_SLACK_S - LAST_MOMENT_S)
    still_open = not waiting.done()
    await clock.tick(LAST_MOMENT_S)

    assert still_open
    with pytest.raises(PullUnanswered):
        await waiting
    assert request.reads == 1


async def test_the_pull_drives_the_request_subject_inbox_and_queue_of_the_real_client() -> None:
    client = Client()
    sent: list[tuple[str, dict[str, object], str]] = []

    async def publish(subject: str, payload: bytes = b"", reply: str = "") -> None:
        sent.append((subject, json.loads(payload), reply))

    client.publish = publish
    inbox = Subscription(client, 1, INBOX)
    bound = JetStreamContext.PullSubscription(
        client.jetstream(), inbox, "STREAM", "durable", INBOX.encode()
    )
    for msg in (
        Msg(client, subject="lane.task", data=b"task"),
        Msg(client, subject=INBOX, headers={"Status": "408", "Description": "Request Timeout"}),
    ):
        inbox._pending_queue.put_nowait(msg)
        inbox._pending_size += len(msg.data)

    taken = [msg.data async for msg in pulled(bound, 16, EXPIRES_S, MonotonicClock())]

    assert taken == [b"task"]
    assert sent == [(NEXT_SUBJECT, {"batch": 16, "expires": int(EXPIRES_S * NANOS_PER_S)}, INBOX)]
    assert inbox._pending_queue.empty()
    assert inbox._pending_size == 0
