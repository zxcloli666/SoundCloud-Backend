from __future__ import annotations

import asyncio

import nats.errors
import pytest

from tests.bus.conftest import AUDIO_TASK, Harness
from worker.bus.lease import (
    ACK_ATTEMPTS,
    ACK_RETRY_PAUSE_S,
    ACK_TIMEOUT_S,
    LastDelivery,
    Settled,
    current_lease,
    drop_if_stale,
)
from worker.domain.outcome import LeaseDropped, Status


def failing_acks(monkeypatch: pytest.MonkeyPatch, msg: object, error: Exception) -> None:
    async def ack_sync(timeout: float = 1.0) -> None:
        raise error

    monkeypatch.setattr(msg, "ack_sync", ack_sync)


async def attach_first(harness: Harness, msg_id: str | None = "task:1"):
    harness.enqueue(AUDIO_TASK, msg_id)
    [msg] = await harness.fetch()
    attached = harness.leases.attach(msg, AUDIO_TASK, harness.lane.correlation_key(AUDIO_TASK))
    return msg, attached


async def beating(lease) -> asyncio.Task[None]:
    task = asyncio.create_task(lease.heartbeat())
    await asyncio.sleep(0)
    return task


async def test_lease_is_keyed_by_stream_seq_and_adopts_newest_delivery(harness: Harness) -> None:
    msg, attached = await attach_first(harness)
    lease = attached.lease
    assert not attached.redelivered
    assert lease.stream_seq == msg.metadata.sequence.stream
    assert lease.num_delivered == 1
    assert lease.msg_id == "task:1"
    assert lease.correlation == "index_audio:42:1:1"
    await harness.clock.tick(harness.lane.ack_wait_s + 1)
    [again] = await harness.fetch()
    assert again.metadata.num_delivered == 2
    readopted = harness.leases.attach(again, AUDIO_TASK, lease.correlation)
    assert readopted.redelivered
    assert readopted.lease is lease
    assert lease.num_delivered == 2
    assert not lease.is_stale
    assert harness.counters.value("redelivered_to_owner_total", lane="audio") == 1
    await lease.ack()
    assert harness.bus.acks("ack_sync")[0].num_delivered == 2


async def test_heartbeat_sends_wpi_only_while_connected_and_trusted(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    beat = await beating(lease)
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert len(harness.bus.acks("in_progress")) == 1
    await harness.bus.disconnect()
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert len(harness.bus.acks("in_progress")) == 1
    await harness.bus.reconnect()
    harness.connection.suspect("puback")
    harness.bus.api_faults.extend([nats.errors.TimeoutError()] * 3)
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert len(harness.bus.acks("in_progress")) == 1
    harness.connection.trust()
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert len(harness.bus.acks("in_progress")) == 2
    lease.settle(Settled.DROPPED)
    await harness.clock.tick(harness.lane.heartbeat_s)
    await beat
    assert len(harness.bus.acks("in_progress")) == 2


async def test_heartbeat_failure_is_counted_and_logged(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    beat = await beating(lease)
    harness.bus.closed = True
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert harness.counters.value("heartbeat_failures_total", lane="audio") == 1
    harness.bus.closed = False
    lease.settle(Settled.DROPPED)
    await harness.clock.tick(harness.lane.heartbeat_s)
    await beat


async def test_silence_longer_than_ack_wait_minus_heartbeat_makes_lease_stale(
    harness: Harness,
) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    harness.clock.advance(harness.lane.ack_wait_s - harness.lane.heartbeat_s)
    assert not lease.is_stale
    harness.clock.advance(0.5)
    assert lease.is_stale
    assert harness.counters.value("lease_lost_total", lane="audio") == 1
    assert lease.is_stale
    assert harness.counters.value("lease_lost_total", lane="audio") == 1


async def test_outage_longer_than_window_with_buffered_wpi_makes_lease_stale(
    harness: Harness,
) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    beat = await beating(lease)
    await harness.clock.tick(harness.lane.heartbeat_s)
    await harness.bus.disconnect()
    await harness.clock.tick(harness.lane.ack_wait_s - harness.lane.heartbeat_s + 1)
    await harness.bus.reconnect()
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert lease.is_stale
    assert [sent.kind for sent in harness.bus.sent] == ["in_progress"]
    await lease.release_transient(30.0)
    assert harness.bus.acks("nak") == []
    assert harness.counters.value("stale_transient_total", lane="audio") == 1
    assert lease.settled.result() is Settled.DROPPED
    await harness.clock.tick(harness.lane.heartbeat_s)
    await beat


async def test_stale_lease_still_acks_after_terminal_publish(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    harness.clock.advance(harness.lane.ack_wait_s)
    assert lease.is_stale
    assert await lease.ack()
    assert len(harness.bus.acks("ack_sync")) == 1


async def test_release_transient_naks_with_delay_and_counts(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    await lease.release_transient(harness.lane.nak_delay(lease.num_delivered))
    [nak] = harness.bus.acks("nak")
    assert nak.delay == 30.0
    assert harness.counters.value("naks_total", lane="audio") == 1
    assert lease.settled.result() is Settled.NACKED
    with pytest.raises(RuntimeError):
        await lease.release_transient(1.0)
    with pytest.raises(RuntimeError):
        await lease.ack()


async def test_suspicious_connection_forbids_nak(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    harness.connection.suspect("ack_sync")
    await lease.release_transient(30.0)
    assert harness.bus.acks("nak") == []
    assert lease.settled.result() is Settled.DROPPED


async def test_disconnected_connection_forbids_a_buffered_nak(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    await harness.bus.disconnect()
    await lease.release_transient(30.0)
    assert lease.settled.result() is Settled.DROPPED
    assert harness.counters.value("stale_transient_total", lane="audio") == 1
    await harness.bus.reconnect()
    assert harness.bus.acks("nak") == []


async def test_abandoned_ack_does_not_silence_heartbeats_of_long_leases(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    _, long_running = await attach_first(harness, "task:long")
    lease = long_running.lease
    beat = await beating(lease)
    finished_msg, finished = await attach_first(harness, "task:done")
    failing_acks(monkeypatch, finished_msg, nats.errors.TimeoutError())
    assert not await finished.lease.ack()
    assert harness.connection.suspicious
    for _ in range(round(harness.lane.ack_wait_s / harness.lane.heartbeat_s) * 2):
        await harness.clock.tick(harness.lane.heartbeat_s)
    assert not harness.connection.suspicious
    assert not lease.is_stale
    assert len(harness.bus.acks("in_progress")) >= 2
    lease.settle(Settled.DROPPED)
    await harness.clock.tick(harness.lane.heartbeat_s)
    await beat


async def test_ack_sync_timeouts_while_connected_mark_suspicious_and_recover(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    msg, attached = await attach_first(harness)
    failing_acks(monkeypatch, msg, nats.errors.TimeoutError())
    assert not await attached.lease.ack()
    assert harness.counters.value("ack_failures_total", lane="audio") == ACK_ATTEMPTS
    assert harness.connection.suspicious
    _, second = await attach_first(harness, "task:2")
    assert await second.lease.ack()
    assert not harness.connection.suspicious


async def test_ack_waits_for_the_connection_instead_of_spending_attempts(
    harness: Harness,
) -> None:
    _, attached = await attach_first(harness)
    await harness.bus.disconnect()
    acking = asyncio.create_task(attached.lease.ack())
    await harness.clock.tick(ACK_TIMEOUT_S)
    assert not acking.done()
    await harness.bus.reconnect()
    await harness.clock.tick(0)
    assert await acking
    assert harness.counters.value("ack_failures_total", lane="audio") == 0
    assert not harness.connection.suspicious
    assert len(harness.bus.acks("ack_sync")) == 1


async def test_ack_is_abandoned_when_the_outage_outlasts_its_budget(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    await harness.bus.disconnect()
    acking = asyncio.create_task(attached.lease.ack())
    await harness.settle()
    await harness.clock.tick(ACK_TIMEOUT_S * ACK_ATTEMPTS - 1)
    assert not acking.done()
    await harness.clock.tick(1)
    assert await acking is False
    assert harness.counters.value("ack_failures_total", lane="audio") == 0
    assert not harness.connection.suspicious


async def test_ack_pauses_between_attempts_after_client_errors(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    msg, attached = await attach_first(harness)
    failing_acks(monkeypatch, msg, nats.errors.OutboundBufferLimitError())
    acking = asyncio.create_task(attached.lease.ack())
    await harness.clock.tick(0)
    assert harness.counters.value("ack_failures_total", lane="audio") == 1
    await harness.clock.tick(ACK_RETRY_PAUSE_S)
    assert harness.counters.value("ack_failures_total", lane="audio") == 2
    await harness.clock.tick(ACK_RETRY_PAUSE_S)
    assert await acking is False
    assert harness.counters.value("ack_failures_total", lane="audio") == ACK_ATTEMPTS
    assert not harness.connection.suspicious


async def test_release_transient_refuses_to_nak_the_last_delivery(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    harness.watch.max_deliver = 1
    with pytest.raises(LastDelivery):
        await attached.lease.release_transient(30.0)
    assert harness.bus.acks("nak") == []
    assert not attached.lease.is_settled


async def test_is_last_delivery_uses_consumer_max_deliver(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    assert not lease.is_last_delivery
    harness.watch.max_deliver = 1
    assert lease.is_last_delivery
    harness.watch.max_deliver = -1
    assert not lease.is_last_delivery


async def test_done_msg_id_carries_correlation_seq_and_status(harness: Harness) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    assert lease.done_msg_id(Status.OK) == f"done.audio:index_audio:42:1:1:{lease.stream_seq}:ok"
    assert lease.done_msg_id(Status.FAILED).endswith(":failed")


async def test_drop_if_stale_raises_only_for_stale_current_lease(harness: Harness) -> None:
    drop_if_stale()
    _, attached = await attach_first(harness)
    lease = attached.lease
    token = current_lease.set(lease)
    try:
        drop_if_stale()
        harness.clock.advance(harness.lane.ack_wait_s)
        with pytest.raises(LeaseDropped):
            drop_if_stale()
    finally:
        current_lease.reset(token)


async def test_settled_lease_is_forgotten_and_a_redelivery_opens_a_new_one(
    harness: Harness,
) -> None:
    _, attached = await attach_first(harness)
    lease = attached.lease
    await lease.release_transient(1.0)
    await harness.clock.tick(0)
    assert len(harness.leases) == 0
    await harness.clock.tick(2)
    [again] = await harness.fetch()
    fresh = harness.leases.attach(again, AUDIO_TASK, lease.correlation)
    assert not fresh.redelivered
    assert fresh.lease is not lease
    assert fresh.lease.num_delivered == 2
