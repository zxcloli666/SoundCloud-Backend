from __future__ import annotations

import asyncio
import math

import nats.errors
import nats.js.errors
import numpy as np
import pytest

from tests.bus.conftest import Harness
from worker.bus.outbox import (
    BACKOFF_CAP_S,
    ConnectionClosed,
    PublishRejected,
    PublishTimedOut,
    encode,
    is_permanent,
)


async def publish(harness: Harness, give_up: float | None = None, body: bytes = b"{}"):
    return await harness.outbox.publish(
        "done.index_audio", body, harness.outbox.headers("done:x", 1), "audio", give_up
    )


async def test_puback_success_trusts_connection_and_frees_room(harness: Harness) -> None:
    harness.connection.suspect("puback")
    ack = await publish(harness)
    assert ack.stream == "PIPELINE_DONE" and not ack.duplicate
    assert not harness.connection.suspicious
    assert harness.outbox.pending == 0 and harness.outbox.pending_bytes == 0
    assert harness.outbox.has_room
    assert harness.bus.published[0][2]["X-Worker-Build"] == "test-build"


async def test_duplicate_puback_is_success(harness: Harness) -> None:
    await publish(harness)
    ack = await publish(harness)
    assert ack.duplicate


@pytest.mark.parametrize(
    "fault",
    [
        nats.js.errors.NoStreamResponseError(),
        nats.js.errors.ServiceUnavailableError(code=503),
        nats.errors.TimeoutError(),
        nats.js.errors.APIError(code=500, err_code=10077),
        nats.js.errors.NotFoundError(code=404),
    ],
)
async def test_transient_faults_are_retried_with_backoff(
    harness: Harness, fault: Exception
) -> None:
    harness.bus.publish_faults.extend([fault, fault])
    task = asyncio.create_task(publish(harness))
    await harness.settle()
    assert harness.outbox.pending == 1
    assert not harness.outbox.has_room or harness.outbox.pending < 256
    await harness.clock.tick(1)
    await harness.clock.tick(2)
    ack = await task
    assert not ack.duplicate
    assert harness.counters.value("publish_failures_total", lane="audio") == 2


async def test_puback_timeout_marks_connection_suspicious(harness: Harness) -> None:
    harness.bus.lost_pubacks = 1
    task = asyncio.create_task(publish(harness))
    await harness.settle()
    assert harness.connection.suspicious
    await harness.clock.tick(1)
    await task
    assert not harness.connection.suspicious
    assert len(harness.bus.published) == 2


async def test_gives_up_after_deadline_when_asked(harness: Harness) -> None:
    await harness.bus.disconnect()
    task = asyncio.create_task(publish(harness, give_up=10.0))
    await harness.settle()
    for _ in range(4):
        await harness.clock.tick(4)
    with pytest.raises(PublishTimedOut):
        await task


async def test_never_gives_up_without_deadline_and_caps_backoff(harness: Harness) -> None:
    harness.bus.publish_faults.extend([nats.js.errors.NoStreamResponseError()] * 13)
    task = asyncio.create_task(publish(harness, give_up=None))
    await harness.settle()
    for _ in range(12):
        await harness.clock.tick(BACKOFF_CAP_S)
    assert not task.done()
    assert harness.clock.sleeping == 1
    await harness.clock.tick(BACKOFF_CAP_S)
    await task
    assert harness.counters.value("publish_failures_total", lane="audio") == 13


async def test_disconnected_publish_waits_for_the_connection_without_suspecting(
    harness: Harness,
) -> None:
    await harness.bus.disconnect()
    task = asyncio.create_task(publish(harness, give_up=None))
    for _ in range(3):
        await harness.clock.tick(BACKOFF_CAP_S)
    assert not task.done()
    assert harness.counters.value("publish_failures_total", lane="audio") == 0
    assert not harness.connection.suspicious
    await harness.bus.reconnect()
    await harness.clock.tick(0)
    ack = await task
    assert not ack.duplicate
    assert len(harness.bus.published) == 1


async def test_closing_while_disconnected_ends_the_wait(harness: Harness) -> None:
    await harness.bus.disconnect()
    task = asyncio.create_task(publish(harness, give_up=None))
    await harness.settle()
    await harness.bus.close()
    with pytest.raises(ConnectionClosed):
        await task


@pytest.mark.parametrize(
    "fault",
    [
        nats.errors.MaxPayloadError(),
        nats.js.errors.APIError(code=400, err_code=10003),
        nats.js.errors.BadRequestError(code=400),
        nats.js.errors.APIError(code=500, err_code=10054),
        nats.js.errors.APIError(code=500, err_code=10076),
    ],
)
async def test_permanent_faults_reject_immediately(harness: Harness, fault: Exception) -> None:
    harness.bus.publish_faults.append(fault)
    with pytest.raises(PublishRejected):
        await publish(harness)
    assert harness.counters.value("publish_rejected_total", lane="audio") == 1
    assert harness.outbox.pending == 0


async def test_payload_over_max_payload_is_rejected_by_client(harness: Harness) -> None:
    harness.bus.max_payload = 10
    with pytest.raises(PublishRejected):
        await publish(harness, body=b"x" * 11)


def test_is_permanent_classifies_by_code_and_err_code() -> None:
    assert is_permanent(nats.js.errors.APIError(code=400))
    assert is_permanent(nats.js.errors.APIError(code=500, err_code=10054))
    assert not is_permanent(nats.js.errors.ServiceUnavailableError(code=503))
    assert not is_permanent(nats.js.errors.APIError(code=500, err_code=10077))


async def test_backpressure_closes_room_by_count_and_bytes(harness: Harness) -> None:
    await harness.bus.disconnect()
    tasks = [
        asyncio.create_task(publish(harness, give_up=None))
        for _ in range(harness.outbox._max_results)
    ]
    await harness.settle()
    assert not harness.outbox.has_room
    assert harness.counters._gauges["outbox_pending"][()] == harness.outbox._max_results
    await harness.bus.reconnect()
    await harness.clock.tick(1)
    await asyncio.gather(*tasks)
    assert harness.outbox.has_room
    harness.outbox._max_bytes = 4
    big = asyncio.create_task(publish(harness, body=b"12345"))
    await harness.settle()
    await big
    assert harness.outbox.has_room


async def test_flush_waits_for_pending_results(harness: Harness) -> None:
    assert await harness.outbox.flush(1.0)
    await harness.bus.disconnect()
    task = asyncio.create_task(publish(harness, give_up=None))
    await harness.settle()
    flushing = asyncio.create_task(harness.outbox.flush(5.0))
    await harness.settle()
    await harness.clock.tick(5)
    assert await flushing is False
    await harness.bus.reconnect()
    await harness.clock.tick(BACKOFF_CAP_S)
    await task
    assert await harness.outbox.flush(1.0)


async def test_closed_connection_stops_retrying(harness: Harness) -> None:
    harness.bus.publish_faults.append(nats.errors.TimeoutError())
    harness.connection.closed.set()
    with pytest.raises(PublishTimedOut):
        await publish(harness, give_up=None)


def test_encode_is_compact_utf8_and_rejects_nan() -> None:
    assert encode({"a": "я", "b": [1, 2]}) == '{"a":"я","b":[1,2]}'.encode()
    with pytest.raises(ValueError):
        encode({"x": math.inf})


def test_encode_writes_float32_vectors_at_float32_precision() -> None:
    vector = np.array([0.1, -0.012345679], dtype=np.float32)
    assert encode({"vec": vector}) == b'{"vec":[0.1,-0.012345679]}'


@pytest.mark.parametrize(
    "value",
    [
        np.array([0.0, np.nan], dtype=np.float32),
        {"nested": [np.float32(np.inf)]},
        np.zeros((4, 4), dtype=np.float32)[:, 0],
        object(),
    ],
    ids=["nan-array", "inf-scalar", "non-contiguous", "unsupported"],
)
def test_encode_refuses_what_the_wire_cannot_carry(value: object) -> None:
    with pytest.raises(ValueError):
        encode({"x": value})
