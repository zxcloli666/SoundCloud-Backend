from __future__ import annotations

import asyncio
import dataclasses

import nats.errors
import nats.js.errors
import pytest
from nats.js import api

from tests.bus.conftest import Harness, build_harness
from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeNats
from worker.bus.consumers import (
    BROKER_MAX_DELIVER,
    CHECK_INTERVAL_S,
    EX_CONFIG,
    NOT_SERVED_RETRY_S,
    UNAVAILABLE_RETRY_MAX_S,
    UNAVAILABLE_RETRY_MIN_S,
    ConfigDrift,
    ConsumerWatch,
    LaneState,
    consumer_diff,
    served_lanes,
)
from worker.contract import Contract
from worker.settings import Settings, SettingsError


def consumer_config(contract: Contract, lane: str = "audio", **overrides: object):
    spec = contract.lane(lane)
    config = api.ConsumerConfig(
        durable_name=spec.durable,
        filter_subject=spec.filter_subject,
        ack_policy=api.AckPolicy.EXPLICIT,
        deliver_policy=api.DeliverPolicy.ALL,
        ack_wait=spec.ack_wait_s,
        max_deliver=spec.max_deliver,
        max_ack_pending=spec.max_ack_pending,
    )
    return dataclasses.replace(config, **overrides)


def test_diff_is_empty_for_the_contract_config(contract: Contract) -> None:
    for name, lane in contract.lanes.items():
        assert consumer_diff(lane, consumer_config(contract, name)) == {}


def test_diff_normalises_server_defaults_and_enums(contract: Contract) -> None:
    lane = contract.lane("encode")
    as_server = api.ConsumerConfig.from_response(
        {
            "durable_name": lane.durable,
            "filter_subject": lane.filter_subject,
            "ack_policy": "explicit",
            "deliver_policy": "all",
            "ack_wait": int(lane.ack_wait_s * 1_000_000_000),
            "max_deliver": lane.max_deliver,
            "max_ack_pending": lane.max_ack_pending,
        }
    )
    assert consumer_diff(lane, as_server) == {}
    defaults = api.ConsumerConfig(durable_name=lane.durable, filter_subjects=[lane.filter_subject])
    diff = consumer_diff(lane, defaults)
    assert diff == {
        "max_deliver": (5, -1),
        "max_ack_pending": (256, 1000),
    }


def test_a_public_node_expects_the_broker_to_deliver_each_task_once(contract: Contract) -> None:
    assert contract.public_lanes
    for name in sorted(contract.public_lanes):
        lane = contract.lane(name)
        broker = consumer_config(contract, name, max_deliver=BROKER_MAX_DELIVER)
        private = consumer_config(contract, name)
        assert consumer_diff(lane, broker, public=True) == {}
        assert consumer_diff(lane, broker) == {"max_deliver": (lane.max_deliver, 1)}
        assert consumer_diff(lane, private, public=True) == {"max_deliver": (1, lane.max_deliver)}


async def test_a_public_node_serves_the_broker_consumer_instead_of_exiting_78(
    harness: Harness,
) -> None:
    harness.bus.consumers[("INDEX_AUDIO", "audio-workers")].config.max_deliver = 1
    public = ConsumerWatch(
        harness.lane,
        harness.connection.js,
        harness.connection,
        harness.counters,
        harness.clock,
        False,
        60.0,
        public=True,
    )
    assert public.max_deliver == BROKER_MAX_DELIVER
    assert await public.verify_at_start() is LaneState.SERVING
    assert public.max_deliver == 1
    with pytest.raises(ConfigDrift):
        await harness.watch.verify_at_start()


def test_diff_reports_every_drifted_field(contract: Contract) -> None:
    lane = contract.lane("audio")
    drifted = consumer_config(
        contract,
        ack_wait=30,
        max_deliver=-1,
        ack_policy=api.AckPolicy.NONE,
        filter_subject="index.audio.old",
        deliver_policy=api.DeliverPolicy.NEW,
        max_ack_pending=1,
    )
    assert set(consumer_diff(lane, drifted)) == {
        "ack_wait_s",
        "max_deliver",
        "ack_policy",
        "filter_subject",
        "deliver_policy",
        "max_ack_pending",
    }


async def test_matching_consumer_serves_and_takes_max_deliver_from_server(
    harness: Harness,
) -> None:
    assert harness.watch.state is LaneState.NOT_PROVISIONED
    assert await harness.watch.check() is LaneState.SERVING
    assert harness.watch.max_deliver == 5
    assert harness.watch.retry_after_s == CHECK_INTERVAL_S
    assert harness.watch.last_error is None


async def test_answered_consumer_info_trusts_a_suspicious_connection(harness: Harness) -> None:
    harness.connection.suspect("ack_sync")
    await harness.watch.check()
    assert not harness.connection.suspicious


async def test_mismatch_at_start_exits_78_before_any_fetch(harness: Harness) -> None:
    harness.bus.consumers[("INDEX_AUDIO", "audio-workers")].config.max_deliver = -1
    with pytest.raises(ConfigDrift) as raised:
        await harness.watch.verify_at_start()
    assert raised.value.exit_code == EX_CONFIG == 78
    assert raised.value.diff == {"max_deliver": (5, -1)}
    assert harness.watch.state is LaneState.DRAINING
    assert harness.bus.bound == []
    assert harness.counters.value("consumer_config_mismatch_total", lane="audio") == 1


async def test_mismatch_during_work_sets_draining_and_stops_the_loop(harness: Harness) -> None:
    await harness.watch.verify_at_start()
    loop = asyncio.create_task(harness.watch.run())
    await harness.settle()
    harness.bus.consumers[("INDEX_AUDIO", "audio-workers")].config.ack_wait = 45
    await harness.clock.tick(CHECK_INTERVAL_S)
    await loop
    assert harness.watch.drifted.is_set()
    assert harness.watch.state is LaneState.DRAINING


async def test_missing_consumer_is_not_provisioned_and_retried_each_minute(
    harness: Harness,
) -> None:
    del harness.bus.consumers[("INDEX_AUDIO", "audio-workers")]
    assert await harness.watch.verify_at_start() is LaneState.NOT_PROVISIONED
    assert harness.watch.retry_after_s == CHECK_INTERVAL_S
    loop = asyncio.create_task(harness.watch.run())
    await harness.settle()
    from tests.conftest import provision_like_jobs

    provision_like_jobs(harness.bus, harness.contract)
    await harness.clock.tick(CHECK_INTERVAL_S)
    assert harness.watch.state is LaneState.SERVING
    loop.cancel()
    await asyncio.gather(loop, return_exceptions=True)


async def test_permissions_violation_is_not_served_and_retried_in_ten_minutes(
    harness: Harness,
) -> None:
    harness.bus.consumer_faults["audio-workers"] = "permissions"
    assert await harness.watch.check() is LaneState.NOT_SERVED
    assert harness.watch.retry_after_s == NOT_SERVED_RETRY_S
    assert harness.watch.last_error == "permissions violation"
    del harness.bus.consumer_faults["audio-workers"]
    assert await harness.watch.check() is LaneState.SERVING


async def test_service_unavailable_keeps_prior_state_with_jitter(harness: Harness) -> None:
    await harness.watch.check()
    harness.bus.consumer_faults["audio-workers"] = nats.js.errors.ServiceUnavailableError(code=503)
    assert await harness.watch.check() is LaneState.SERVING
    assert UNAVAILABLE_RETRY_MIN_S <= harness.watch.retry_after_s <= UNAVAILABLE_RETRY_MAX_S
    assert harness.counters.value("consumer_check_failures_total", lane="audio") == 1


async def test_timeout_while_connected_without_denial_keeps_prior_state(
    harness: Harness,
) -> None:
    harness.bus.consumer_faults["audio-workers"] = nats.errors.TimeoutError()
    assert await harness.watch.check() is LaneState.NOT_PROVISIONED
    assert UNAVAILABLE_RETRY_MIN_S <= harness.watch.retry_after_s <= UNAVAILABLE_RETRY_MAX_S


async def test_timeout_while_disconnected_waits_for_reconnect(harness: Harness) -> None:
    await harness.watch.check()
    await harness.bus.disconnect()
    assert await harness.watch.check() is LaneState.SERVING
    assert harness.watch.last_error == "disconnected"
    loop = asyncio.create_task(harness.watch.run())
    await harness.clock.tick(UNAVAILABLE_RETRY_MIN_S)
    await harness.clock.tick(UNAVAILABLE_RETRY_MIN_S)
    assert harness.counters.value("consumer_check_failures_total", lane="audio") == 1
    loop.cancel()
    await asyncio.gather(loop, return_exceptions=True)


async def test_other_api_errors_keep_state_and_retry_in_a_minute(harness: Harness) -> None:
    harness.bus.consumer_faults["audio-workers"] = nats.js.errors.APIError(code=500)
    assert await harness.watch.check() is LaneState.NOT_PROVISIONED
    assert harness.watch.retry_after_s == CHECK_INTERVAL_S


@pytest.mark.parametrize(
    "fault",
    [
        nats.errors.OutboundBufferLimitError(),
        nats.errors.ConnectionClosedError(),
        nats.errors.ConnectionDrainingError(),
    ],
)
async def test_client_errors_keep_state_and_the_watch_alive(
    harness: Harness, fault: Exception
) -> None:
    await harness.watch.check()
    harness.bus.consumer_faults["audio-workers"] = fault
    assert await harness.watch.check() is LaneState.SERVING
    assert harness.watch.retry_after_s == UNAVAILABLE_RETRY_MIN_S
    assert harness.watch.last_error
    loop = asyncio.create_task(harness.watch.run())
    await harness.clock.tick(UNAVAILABLE_RETRY_MIN_S)
    assert not loop.done()
    del harness.bus.consumer_faults["audio-workers"]
    await harness.clock.tick(UNAVAILABLE_RETRY_MIN_S)
    assert harness.watch.retry_after_s == CHECK_INTERVAL_S
    assert harness.counters.value("consumer_check_failures_total", lane="audio") == 2
    loop.cancel()
    await asyncio.gather(loop, return_exceptions=True)


async def test_verify_at_start_survives_client_errors(harness: Harness) -> None:
    harness.bus.consumer_faults["audio-workers"] = nats.errors.OutboundBufferLimitError()
    assert await harness.watch.verify_at_start() is LaneState.NOT_PROVISIONED


async def test_required_lane_degrades_after_grace_and_counts_once(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    harness = await build_harness(fake_nats, contract, settings, clock, required=True)
    del harness.bus.consumers[("INDEX_AUDIO", "audio-workers")]
    assert await harness.watch.check() is LaneState.NOT_PROVISIONED
    clock.advance(settings.lanes.required_grace_s)
    assert await harness.watch.check() is LaneState.NOT_PROVISIONED
    clock.advance(1)
    assert harness.watch.state is LaneState.DEGRADED
    assert await harness.watch.check() is LaneState.DEGRADED
    assert await harness.watch.check() is LaneState.DEGRADED
    assert harness.counters.value("lane_required_missing_total", lane="audio") == 1
    from tests.conftest import provision_like_jobs

    provision_like_jobs(harness.bus, harness.contract)
    assert await harness.watch.check() is LaneState.SERVING
    assert harness.runner.state is LaneState.SERVING
    await harness.stop()


async def test_optional_lane_never_degrades(harness: Harness) -> None:
    del harness.bus.consumers[("INDEX_AUDIO", "audio-workers")]
    await harness.watch.check()
    harness.clock.advance(10_000)
    assert await harness.watch.check() is LaneState.NOT_PROVISIONED


async def test_paused_and_publish_blocked_flags(harness: Harness) -> None:
    await harness.watch.check()
    harness.watch.paused = True
    assert harness.watch.state is LaneState.PAUSED
    harness.watch.publish_blocked = True
    assert harness.watch.state is LaneState.DEGRADED
    harness.watch.publish_blocked = False
    harness.watch.paused = False
    assert harness.watch.state is LaneState.SERVING


async def test_broken_engine_degrades_even_a_paused_lane(harness: Harness) -> None:
    await harness.watch.check()
    harness.watch.paused = True
    harness.watch.engine_broken = True
    assert harness.watch.state is LaneState.DEGRADED
    harness.watch.engine_broken = False
    assert harness.watch.state is LaneState.PAUSED


def test_second_lock_rejects_closed_lanes_on_public_nodes(
    settings: Settings, contract: Contract
) -> None:
    assert [lane.name for lane in served_lanes(settings, contract)] == list(settings.lanes.enabled)
    public = dataclasses.replace(
        settings,
        worker=dataclasses.replace(settings.worker, trust="public"),
        lanes=dataclasses.replace(settings.lanes, enabled=("audio", "encode")),
    )
    with pytest.raises(SettingsError, match="encode"):
        served_lanes(public, contract)
    allowed = dataclasses.replace(
        public, lanes=dataclasses.replace(settings.lanes, enabled=("audio", "transcribe"))
    )
    assert [lane.name for lane in served_lanes(allowed, contract)] == ["audio", "transcribe"]
