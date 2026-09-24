from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import math
from collections.abc import Awaitable, Callable, Mapping

import nats.errors
import nats.js.errors
import pytest

from tests.bus.conftest import AUDIO_TASK, ENCODE_TASK, Harness, build_harness
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.jetstream import FakeNats
from worker.bus.consumers import LaneState
from worker.bus.lane_runner import (
    DEGRADED_PROBE_S,
    DRAIN_NAK_DELAY_S,
    IDLE_POLL_S,
    PUBLISH_GIVE_UP_S,
)
from worker.bus.lease import Settled, drop_if_stale
from worker.bus.outbox import PublishTimedOut
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.encode_text import EncodeTextLane
from worker.domain.outcome import (
    LeaseDropped,
    Outcome,
    PermanentFailure,
    Reason,
    TransientFailure,
)
from worker.domain.ports import EngineUnavailable
from worker.settings import Settings


def transient(reason: Reason = Reason.DOWNLOAD_FAILED):
    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        raise TransientFailure(reason, "scripted")

    return behaviour


def permanent(reason: Reason = Reason.UNDECODABLE_AUDIO):
    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        raise PermanentFailure(reason, "scripted")

    return behaviour


def crashing():
    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        raise KeyError("bug")

    return behaviour


def dropping():
    async def behaviour(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        drop_if_stale()
        raise LeaseDropped("scripted")

    return behaviour


async def redeliver(harness: Harness, times: int) -> None:
    for _ in range(times):
        await harness.clock.tick((harness.lane.nak_cap_s or 0) + harness.lane.ack_wait_s + 1)
        [msg] = await harness.fetch()
        await harness.handle(msg)


def done_bodies(harness: Harness) -> list[dict[str, object]]:
    return [json.loads(body) for _, body, _ in harness.done_published()]


async def test_ok_publishes_done_before_ack_sync_with_msg_id(harness: Harness) -> None:
    seq = harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [(subject, body, headers)] = harness.done_published()
    assert subject == "done.index_audio"
    done = json.loads(body)
    assert done["status"] == "ok"
    assert done["sc_track_id"] == "42"
    assert done["producer"]["worker_id"] == "test-worker"
    assert headers["Nats-Msg-Id"] == f"done.audio:index_audio:42:1:1:{seq}:ok"
    assert headers["X-Worker-Id"] == "test-worker"
    assert headers["X-Deliveries"] == "1"
    [ack] = harness.bus.acks()
    assert ack.kind == "ack_sync" and ack.seq == seq
    assert seq not in harness.bus.streams["INDEX_AUDIO"].messages
    assert harness.counters.value("done_total", lane="audio", status="ok") == 1
    assert harness.contract.validate("done.index_audio", done) == []


async def test_transient_failure_naks_with_lane_delay_on_non_last_delivery(
    harness: Harness,
) -> None:
    harness.processor.behaviour = transient()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.done_published() == []
    [nak] = harness.bus.acks()
    assert nak.kind == "nak" and nak.delay == harness.lane.nak_delay(1) == 30.0
    await harness.clock.tick(31)
    [again] = await harness.fetch()
    assert again.metadata.num_delivered == 2
    await harness.handle(again)
    assert harness.bus.acks("nak")[-1].delay == 60.0


async def test_transient_failure_on_last_delivery_publishes_failed_and_acks(
    harness: Harness,
) -> None:
    harness.processor.behaviour = transient(Reason.ENGINE_CRASHED)
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    await redeliver(harness, 4)
    assert [sent.kind for sent in harness.bus.sent] == ["nak"] * 4 + ["ack_sync"]
    [done] = done_bodies(harness)
    assert done["status"] == "failed" and done["reason"] == "engine_crashed"
    assert harness.done_published()[0][2]["X-Deliveries"] == "5"
    assert harness.contract.validate("done.index_audio", done) == []


async def test_permanent_failure_publishes_immediately_without_retries(harness: Harness) -> None:
    harness.processor.behaviour = permanent()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [done] = done_bodies(harness)
    assert done["status"] == "failed" and done["reason"] == "undecodable_audio"
    assert [sent.kind for sent in harness.bus.sent] == ["ack_sync"]


async def test_unexpected_exception_is_a_counted_internal_error(harness: Harness) -> None:
    harness.processor.behaviour = crashing()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.counters.value("process_crashes_total", lane="audio") == 1
    assert harness.bus.acks("nak")[0].delay == 30.0
    harness.watch.max_deliver = 2
    await redeliver(harness, 1)
    [done] = done_bodies(harness)
    assert done["reason"] == "internal_error" and "KeyError" in done["detail"]


async def test_lease_dropped_sends_nothing(harness: Harness) -> None:
    harness.processor.behaviour = dropping()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.bus.sent == []
    assert harness.done_published() == []
    assert len(harness.leases) == 0


async def test_a_real_lane_hitting_a_stale_lease_is_dropped_not_failed(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, "encode")
    engines = FakeEngines()

    def stale_before_queue(**kwargs: object) -> object:
        harness.clock.advance(harness.lane.ack_wait_s)
        drop_if_stale()
        return None

    engines.overrides["embed_text"] = stale_before_queue
    harness.processor.behaviour = EncodeTextLane(engines, harness.counters).process
    text = "hello"
    harness.enqueue({"model": "lyrics", "text": text, "hash": hashlib.sha256(b"hello").hexdigest()})
    [msg] = await harness.fetch()
    try:
        await harness.handle(msg)
        assert harness.done_published() == []
        assert len(harness.leases) == 0
        assert harness.counters.total("lane_internal_errors_total") == 0
        assert engines.calls[0] == ("embed_text", {"texts": [text], "kind": "query"})
    finally:
        await harness.stop()


async def test_a_transient_outcome_returned_by_a_real_lane_is_retried_then_published(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, "encode")
    engines = FakeEngines()
    engines.fail("embed_text", EngineUnavailable("text", "restarting"))
    harness.processor.behaviour = EncodeTextLane(engines, harness.counters).process
    harness.watch.max_deliver = 2
    harness.enqueue(
        {"model": "lyrics", "text": "hello", "hash": hashlib.sha256(b"hello").hexdigest()}
    )
    [msg] = await harness.fetch()
    try:
        await harness.handle(msg)
        assert harness.done_published() == []
        [nak] = harness.bus.acks()
        assert nak.kind == "nak" and nak.delay == harness.lane.nak_delay(1)
        await redeliver(harness, 1)
        [done] = done_bodies(harness)
        assert done["status"] == "failed" and done["reason"] == "engine_crashed"
        assert done["detail"] == "slot=text state=restarting"
        assert harness.contract.validate("done.encode", done) == []
        assert [sent.kind for sent in harness.bus.sent] == ["nak", "ack_sync"]
    finally:
        await harness.stop()


async def test_a_transient_outcome_on_a_stale_lease_is_dropped_without_nak(
    harness: Harness,
) -> None:
    async def engine_restarted(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        harness.clock.advance(harness.lane.ack_wait_s)
        return Outcome.failed(Reason.OUT_OF_MEMORY, "domain")

    harness.processor.behaviour = engine_restarted
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.done_published() == []
    assert harness.bus.acks("nak") == []
    assert harness.counters.value("stale_transient_total", lane="audio") == 1


async def test_stale_lease_neither_naks_nor_beats_but_publishes_terminal(
    harness: Harness,
) -> None:
    harness.processor.hold()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    task = harness.handle(msg)
    await harness.processor.started.wait()
    await harness.bus.disconnect()
    await harness.clock.tick(harness.lane.ack_wait_s)
    await harness.bus.reconnect()
    await harness.clock.tick(harness.lane.heartbeat_s)
    lease = harness.leases.get(msg.seq)
    assert lease is not None and lease.is_stale
    harness.processor.behaviour = transient()
    harness.processor.resume()
    await task
    assert harness.bus.acks("nak") == []
    assert harness.counters.value("stale_transient_total", lane="audio") == 1
    harness.processor.hold()
    await harness.clock.tick(harness.lane.ack_wait_s)
    [again] = await harness.fetch()
    task = harness.handle(again)
    await harness.processor.started.wait()
    harness.clock.advance(harness.lane.ack_wait_s)
    harness.processor.behaviour = harness.processor._ok
    harness.processor.resume()
    await task
    assert [sent.kind for sent in harness.bus.sent if sent.kind != "in_progress"] == ["ack_sync"]
    assert done_bodies(harness)[0]["status"] == "ok"


async def test_redelivery_to_owner_is_adopted_and_answered_once(harness: Harness) -> None:
    harness.processor.hold()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    task = harness.handle(msg)
    await harness.processor.started.wait()
    await harness.clock.tick(harness.lane.ack_wait_s + 1)
    [again] = await harness.fetch()
    assert again.metadata.num_delivered == 2
    await harness.handle(again)
    assert len(harness.processor.calls) == 1
    harness.processor.resume()
    await task
    assert [sent.kind for sent in harness.bus.sent if sent.kind != "in_progress"] == ["ack_sync"]
    assert harness.bus.acks("ack_sync")[0].num_delivered == 2
    assert harness.done_published()[0][2]["X-Deliveries"] == "2"


async def test_same_correlation_other_seq_joins_and_acks_only_after_publish(
    harness: Harness,
) -> None:
    harness.processor.hold()
    harness.enqueue(AUDIO_TASK, "task:a")
    harness.enqueue(AUDIO_TASK, "task:b")
    first, second = await harness.fetch(batch=2)
    owner = harness.handle(first)
    await harness.processor.started.wait()
    joiner = harness.handle(second)
    await harness.settle()
    assert not joiner.done()
    harness.processor.resume()
    await asyncio.gather(owner, joiner)
    assert len(harness.processor.calls) == 1
    assert len(harness.done_published()) == 1
    assert sorted(sent.seq for sent in harness.bus.acks("ack_sync")) == [first.seq, second.seq]


async def test_joiner_processes_itself_when_owner_nacks(harness: Harness) -> None:
    harness.processor.hold()
    harness.processor.behaviour = transient()
    harness.enqueue(AUDIO_TASK, "task:a")
    harness.enqueue(AUDIO_TASK, "task:b")
    first, second = await harness.fetch(batch=2)
    owner = harness.handle(first)
    await harness.processor.started.wait()
    joiner = harness.handle(second)
    await harness.settle()
    harness.processor.resume()
    await asyncio.gather(owner, joiner)
    assert len(harness.processor.calls) == 2
    assert [sent.kind for sent in harness.bus.sent] == ["nak", "nak"]


async def test_invalid_json_is_termed_with_an_invalid_event(harness: Harness) -> None:
    seq = harness.enqueue(b"{not json", "task:bad")
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [(subject, body, headers)] = harness.bus.published
    assert subject == "worker.invalid.audio"
    event = json.loads(body)
    assert event["stream_seq"] == seq and event["nats_msg_id"] == "task:bad"
    assert base64.b64decode(event["body_base64"]) == b"{not json"
    assert headers["Nats-Msg-Id"] == f"invalid:INDEX_AUDIO:{seq}"
    [term] = harness.bus.acks()
    assert term.kind == "term"
    assert harness.counters.value("invalid_tasks_total", lane="audio") == 1


@pytest.mark.parametrize("constant", ["NaN", "Infinity", "-Infinity"])
async def test_non_finite_literal_in_task_is_invalid(harness: Harness, constant: str) -> None:
    body = json.dumps(AUDIO_TASK).replace('"42"', constant).encode()
    harness.enqueue(body, "task:nan")
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [(subject, event, _)] = harness.bus.published
    assert subject == "worker.invalid.audio"
    assert constant in json.loads(event)["error"]
    assert [sent.kind for sent in harness.bus.acks()] == ["term"]
    assert harness.counters.value("invalid_tasks_total", lane="audio") == 1
    assert harness.counters.value("lease_unsettled_total", lane="audio") == 0


async def test_missing_correlation_field_is_invalid(harness: Harness) -> None:
    harness.enqueue({"sc_track_id": "1"})
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.bus.acks()[0].kind == "term"
    assert "upload_generation" in json.loads(harness.bus.published[0][1])["error"]


async def test_task_without_msg_id_is_counted(harness: Harness) -> None:
    harness.enqueue(AUDIO_TASK, None)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert harness.counters.value("task_without_msg_id_total", lane="audio") == 1
    assert done_bodies(harness)[0]["status"] == "ok"


async def test_publish_gives_up_after_60s_on_non_last_delivery_and_naks(
    harness: Harness,
) -> None:
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.bus.disconnect()
    task = harness.handle(msg)
    await harness.settle()
    assert harness.done_published() == []
    await harness.clock.tick(PUBLISH_GIVE_UP_S + 1)
    await harness.clock.tick(PUBLISH_GIVE_UP_S)
    await harness.bus.reconnect()
    await harness.clock.tick(1)
    await task
    assert harness.counters.value("publish_failures_total", lane="audio") == 0
    assert harness.bus.acks("ack_sync") == []
    assert harness.counters.value("stale_transient_total", lane="audio") == 1


async def test_publish_timeout_after_adopting_the_last_delivery_keeps_publishing(
    harness: Harness,
) -> None:
    harness.watch.max_deliver = 2
    seq = harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    consumer = harness.bus.consumers[(harness.lane.stream, harness.lane.durable)]
    real_publish = harness.outbox.publish
    give_ups: list[object] = []

    async def adopt_then_time_out(*args: object, **kwargs: object):
        give_ups.append(args[4])
        if len(give_ups) == 1:
            consumer.pending[seq].available_at = harness.clock.now()
            [again] = await harness.fetch()
            await harness.handle(again)
            raise PublishTimedOut("scripted")
        return await real_publish(*args, **kwargs)

    harness.outbox.publish = adopt_then_time_out
    await harness.handle(msg)
    assert give_ups == [PUBLISH_GIVE_UP_S, None]
    assert harness.bus.acks("nak") == []
    [ack] = harness.bus.acks("ack_sync")
    assert ack.num_delivered == 2
    assert done_bodies(harness)[0]["status"] == "ok"
    assert harness.done_published()[0][2]["X-Deliveries"] == "2"
    assert harness.counters.value("publish_kept_on_last_delivery_total", lane="audio") == 1


async def test_closed_connection_on_last_delivery_drops_without_looping(harness: Harness) -> None:
    harness.watch.max_deliver = 1
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.bus.disconnect()
    task = harness.handle(msg)
    await harness.settle()
    await harness.bus.close()
    await harness.clock.tick(1)
    await task
    assert harness.done_published() == []
    assert harness.bus.acks("nak") == []
    assert harness.counters.value("stale_transient_total", lane="audio") == 1


async def test_publish_on_last_delivery_waits_for_the_stream(harness: Harness) -> None:
    harness.watch.max_deliver = 1
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    harness.bus.publish_faults.extend([nats.js.errors.NoStreamResponseError()] * 3)
    task = harness.handle(msg)
    await harness.settle()
    for _ in range(3):
        await harness.clock.tick(8)
    await task
    assert harness.counters.value("publish_failures_total", lane="audio") == 3
    assert [sent.kind for sent in harness.bus.sent if sent.kind != "in_progress"] == ["ack_sync"]
    assert done_bodies(harness)[0]["status"] == "ok"


async def test_rejected_publish_falls_back_to_compact_internal_error(harness: Harness) -> None:
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    harness.bus.publish_faults.append(nats.js.errors.APIError(code=500, err_code=10054))
    await harness.handle(msg)
    [done] = done_bodies(harness)
    assert done["status"] == "failed" and done["reason"] == "internal_error"
    assert harness.counters.value("publish_rejected_total", lane="audio") == 1
    assert [sent.kind for sent in harness.bus.sent] == ["ack_sync"]


async def test_rejected_compact_degrades_lane_until_a_probe_succeeds(harness: Harness) -> None:
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    harness.bus.publish_faults.extend(
        [
            nats.errors.MaxPayloadError(),
            nats.js.errors.APIError(code=400),
            nats.js.errors.APIError(code=400),
        ]
    )
    task = harness.handle(msg)
    await harness.settle()
    assert harness.watch.state is LaneState.DEGRADED
    assert harness.runner.state is LaneState.DEGRADED
    assert harness.bus.acks() == []
    await harness.clock.tick(DEGRADED_PROBE_S)
    assert harness.watch.state is LaneState.DEGRADED
    await harness.clock.tick(DEGRADED_PROBE_S)
    await task
    assert harness.watch.state is LaneState.SERVING or harness.watch.state is not LaneState.DEGRADED
    assert done_bodies(harness)[-1]["reason"] == "internal_error"
    assert [sent.kind for sent in harness.bus.sent if sent.kind != "in_progress"] == ["ack_sync"]


async def test_result_over_limit_drops_words_then_fails_model_output_invalid(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, "transcribe")
    big = "x" * contract.lane("transcribe").result_max_bytes
    task = {
        "sc_track_id": "7",
        "upload_generation": 1,
        "attempt": 1,
        "audio_url": "https://a/b",
        "reference_text": "la",
        "reference_lines_total": 1,
        "language": None,
        "mode": "align",
    }

    async def with_words(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        return Outcome.ok(synced_lrc="[00:01.00]la", words=[{"text": big}])

    async def huge(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        return Outcome.ok(synced_lrc=big + big)

    harness.processor.behaviour = with_words
    harness.enqueue(task, "t:1")
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [done] = done_bodies(harness)
    assert done["status"] == "ok" and "words" not in done
    assert harness.counters.value("result_trimmed_total", lane="transcribe") == 1
    harness.processor.behaviour = huge
    harness.enqueue({**task, "attempt": 2}, "t:2")
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert done_bodies(harness)[1]["reason"] == "model_output_invalid"
    await harness.stop()


async def test_non_finite_number_in_result_is_model_output_invalid(harness: Harness) -> None:
    async def nan(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        return Outcome.ok(mert=[math.nan], clap=[0.0])

    harness.processor.behaviour = nan
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    [done] = done_bodies(harness)
    assert done["reason"] == "model_output_invalid"


async def test_deadline_is_fetch_time_plus_lane_deadline(harness: Harness) -> None:
    seen: list[float] = []

    async def record(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        seen.append(deadline.remaining())
        return Outcome.ok(mert=[0.0], clap=[0.0])

    harness.processor.behaviour = record
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.handle(msg)
    assert seen == [harness.lane.deadline_s]


async def test_runner_fetches_up_to_free_capacity_and_drains_lingering(
    harness: Harness,
) -> None:
    harness.processor.hold()
    for index in range(4):
        harness.enqueue({**AUDIO_TASK, "attempt": index + 1}, f"task:{index}")
    harness.watch.max_deliver = 5
    await harness.watch.check()
    assert harness.watch.state is LaneState.SERVING
    harness.start()
    await harness.settle()
    assert harness.runner.inflight == 2
    [subscription] = harness.bus.bound
    assert subscription.fetches[0][0] == 2
    seqs = sorted(harness.bus.streams["INDEX_AUDIO"].messages)
    harness.bus.linger(subscription, seqs[2])
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert harness.runner.inflight == 3
    assert harness.counters.value("lingering_taken_total", lane="audio") == 1
    assert len(harness.bus.acks("in_progress")) >= 1
    harness.processor.resume()
    await harness.clock.tick(harness.lane.heartbeat_s)
    await harness.clock.tick(harness.lane.heartbeat_s)
    await harness.settle()
    assert len(harness.processor.calls) == 4
    assert harness.runner.inflight == 0


async def test_runner_stops_fetching_when_lane_is_not_serving_or_outbox_full(
    harness: Harness,
) -> None:
    harness.enqueue(AUDIO_TASK)
    harness.start()
    await harness.clock.tick(2)
    assert harness.bus.bound == []
    await harness.watch.check()
    harness.outbox.pending = harness.outbox._max_results
    assert harness.runner.state is LaneState.PAUSED
    await harness.clock.tick(2)
    assert harness.bus.bound == []
    harness.outbox.pending = 0
    await harness.clock.tick(2)
    assert len(harness.processor.calls) == 1


async def test_drain_naks_early_deliveries_with_five_seconds(harness: Harness) -> None:
    harness.processor.hold()
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    await harness.watch.check()
    harness.runner._spawn(msg)
    await harness.processor.started.wait()
    drained = asyncio.create_task(harness.runner.drain(grace_s=5))
    await harness.settle()
    await harness.clock.tick(5)
    await harness.clock.tick(1)
    assert await drained == 0
    [nak] = harness.bus.acks("nak")
    assert nak.delay == DRAIN_NAK_DELAY_S
    assert harness.done_published() == []


@pytest.mark.parametrize(
    ("lane_name", "task", "reason"),
    [
        ("audio", AUDIO_TASK, "engine_restarted"),
        ("encode", ENCODE_TASK, "deadline_exceeded"),
    ],
)
async def test_drain_on_late_delivery_publishes_failed_then_acks(
    fake_nats: FakeNats,
    contract: Contract,
    settings: Settings,
    clock: FakeClock,
    lane_name: str,
    task: dict[str, object],
    reason: str,
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, lane_name)
    harness.processor.hold()
    harness.enqueue(task)
    await harness.watch.check()
    for _ in range(3):
        [msg] = await harness.fetch()
        await msg.nak(0)
        await harness.clock.tick(0.1)
    [msg] = await harness.fetch()
    assert msg.metadata.num_delivered == 4
    harness.runner._spawn(msg)
    await harness.processor.started.wait()
    drained = asyncio.create_task(harness.runner.drain(grace_s=1))
    await harness.settle()
    await harness.clock.tick(1)
    await harness.clock.tick(1)
    assert await drained == 0
    [done] = done_bodies(harness)
    assert done["status"] == "failed" and done["reason"] == reason
    assert harness.bus.acks("ack_sync")[0].seq == msg.seq
    await harness.stop()


async def test_stop_fetching_reports_draining_in_the_status_snapshot(harness: Harness) -> None:
    await harness.watch.check()
    assert harness.runner.state is LaneState.SERVING
    harness.runner.stop_fetching()
    assert harness.runner.state is LaneState.DRAINING
    assert harness.runner.snapshot()["state"] == "draining"


async def test_drift_during_work_stops_fetch_and_reports_draining(harness: Harness) -> None:
    await harness.watch.check()
    harness.start()
    await harness.settle()
    harness.bus.consumers[("INDEX_AUDIO", "audio-workers")].config.ack_wait = 61
    await harness.watch.check()
    assert harness.watch.drifted.is_set()
    assert harness.runner.state is LaneState.DRAINING
    await harness.settle()
    [subscription] = harness.bus.bound
    fetches = len(subscription.fetches)
    harness.enqueue(AUDIO_TASK)
    await harness.clock.tick(5)
    await harness.clock.tick(5)
    assert harness.processor.calls == []
    assert len(subscription.fetches) == fetches


async def test_handler_crash_is_counted_and_lease_settled(harness: Harness) -> None:
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()
    harness.leases.attach = None
    harness.runner._spawn(msg)
    await harness.settle()
    assert harness.counters.value("handler_crashes_total", lane="audio") == 1


async def test_unsettled_lease_is_dropped_on_exit(harness: Harness) -> None:
    harness.enqueue(AUDIO_TASK)
    [msg] = await harness.fetch()

    async def broken(*args: object, **kwargs: object) -> None:
        raise RuntimeError("outbox bug")

    harness.outbox.publish = broken
    with pytest.raises(RuntimeError):
        await harness.handler.handle(msg, asyncio.Event())
    await harness.settle()
    assert harness.counters.value("lease_unsettled_total", lane="audio") == 1
    assert len(harness.leases) == 0
    assert len(harness.inflight) == 0
    assert harness.bus.acks() == []


async def test_settled_marker_values() -> None:
    assert {s.value for s in Settled} == {"published", "nacked", "dropped"}


async def test_status_left_in_a_full_lane_queue_is_skipped_without_a_new_pull(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, capacity=1)
    harness.processor.hold()
    harness.enqueue(AUDIO_TASK, "task:1")
    await harness.watch.check()
    harness.start()
    await harness.settle()
    assert harness.runner.inflight == 1
    [subscription] = harness.bus.bound
    fetches = len(subscription.fetches)
    harness.enqueue({**AUDIO_TASK, "attempt": 2}, "task:2")
    harness.bus.linger_status(subscription, "408")
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert subscription.pending_msgs == 0
    assert len(subscription.fetches) == fetches
    assert harness.runner.inflight == 1
    assert harness.counters.value("lingering_status_total", lane="audio") == 1
    await harness.stop()


async def test_lingering_message_gets_a_lease_while_the_lane_is_paused(harness: Harness) -> None:
    harness.processor.hold()
    await harness.watch.check()
    harness.start()
    await harness.settle()
    [subscription] = harness.bus.bound
    harness.outbox.pending = harness.outbox._max_results
    assert harness.runner.state is LaneState.PAUSED
    await harness.clock.tick(IDLE_POLL_S)
    harness.bus.linger(subscription, harness.enqueue(AUDIO_TASK))
    await harness.clock.tick(IDLE_POLL_S)
    assert len(harness.processor.calls) == 1
    assert len(harness.leases) == 1
    await harness.clock.tick(harness.lane.heartbeat_s)
    assert len(harness.bus.acks("in_progress")) == 1


async def test_lingering_message_is_not_lost_to_a_heartbeat_pull(harness: Harness) -> None:
    harness.processor.hold()
    await harness.watch.check()
    harness.start()
    await harness.settle()
    [subscription] = harness.bus.bound
    harness.bus.linger(subscription, harness.enqueue(AUDIO_TASK))
    await harness.settle()
    assert len(harness.processor.calls) == 1
    assert harness.counters.value("fetch_failures_total", lane="audio") == 0


QUEUE_LANES = ("audio", "lyrics", "transcribe", "encode", "collab", "taste")


@pytest.mark.parametrize("lane_name", QUEUE_LANES)
async def test_drain_failure_built_by_the_bus_matches_the_lane_schema(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock, lane_name: str
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, lane_name)
    harness.processor.hold()
    harness.enqueue(dict(samples.REQUESTS[lane_name]))
    await harness.watch.check()
    for _ in range(harness.watch.max_deliver - 2):
        [msg] = await harness.fetch()
        await msg.nak(0)
        await harness.clock.tick(0.1)
    [msg] = await harness.fetch()
    harness.runner._spawn(msg)
    await harness.processor.started.wait()
    drained = asyncio.create_task(harness.runner.drain(grace_s=1))
    await harness.settle()
    await harness.clock.tick(1)
    await harness.clock.tick(1)
    assert await drained == 0
    [done] = done_bodies(harness)
    assert done["status"] == "failed"
    assert contract.validate(str(harness.lane.done_subject), done) == []
    await harness.stop()


@pytest.mark.parametrize("lane_name", QUEUE_LANES)
async def test_compact_rejection_failure_matches_the_lane_schema(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock, lane_name: str
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, lane_name)
    harness.enqueue(dict(samples.REQUESTS[lane_name]))
    [msg] = await harness.fetch()
    harness.bus.publish_faults.append(nats.js.errors.APIError(code=500, err_code=10054))
    await harness.handle(msg)
    [done] = done_bodies(harness)
    assert done["reason"] == "internal_error"
    assert contract.validate(str(harness.lane.done_subject), done) == []
    await harness.stop()


async def oversized(request: Mapping[str, object], deadline: Deadline) -> Outcome:
    return Outcome.ok(blob="x" * (8 << 20))


@pytest.mark.parametrize("lane_name", QUEUE_LANES)
@pytest.mark.parametrize(
    ("behaviour", "reason"),
    [(crashing(), "internal_error"), (oversized, "model_output_invalid")],
    ids=["crash", "oversized"],
)
async def test_last_delivery_failure_built_by_the_bus_matches_the_lane_schema(
    fake_nats: FakeNats,
    contract: Contract,
    settings: Settings,
    clock: FakeClock,
    lane_name: str,
    behaviour: Callable[[Mapping[str, object], Deadline], Awaitable[Outcome]],
    reason: str,
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, lane_name)
    harness.processor.behaviour = behaviour
    harness.watch.max_deliver = 1
    harness.enqueue(dict(samples.REQUESTS[lane_name]))
    [msg] = await harness.fetch()
    try:
        await harness.handle(msg)
        [done] = done_bodies(harness)
        assert done["status"] == "failed" and done["reason"] == reason
        assert contract.validate(str(harness.lane.done_subject), done) == []
    finally:
        await harness.stop()
