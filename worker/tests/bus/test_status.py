from __future__ import annotations

import asyncio
import dataclasses
import json

from nats.aio.msg import Msg

from tests.bus.conftest import Harness
from worker.bus.status import STATUS_INTERVAL_S, StatusReporter, parse_labels
from worker.settings import Settings


def reporter(harness: Harness, settings: Settings) -> StatusReporter:
    return StatusReporter(
        harness.connection,
        harness.contract,
        settings,
        harness.counters,
        harness.clock,
        {"audio": harness.runner},
        harness.outbox,
        "s4.abcdef12.12345678.deadbeef",
        lambda: {"muq": {"state": "ready", "restarts": 0}},
        lambda: {"name": "RTX 3060", "total_mib": 12288, "used_mib": 1024},
    )


async def test_snapshot_has_the_documented_shape(harness: Harness, settings: Settings) -> None:
    harness.counters.inc("llm_calls_total", provider="anthropic", outcome="ok")
    harness.counters.inc("align_engine_total", engine="mms")
    harness.counters.observe("llm_latency_ms", 120.0, provider="anthropic")
    harness.counters.inc("lease_lost_total", lane="audio")
    status = reporter(harness, settings)
    harness.clock.advance(5)
    snapshot = status.snapshot()
    assert snapshot["worker_id"] == "test-worker" and snapshot["build"] == "dev"
    assert snapshot["trust"] == "trusted" and snapshot["profile"] == ""
    assert snapshot["sync_version"] == "s4.abcdef12.12345678.deadbeef"
    assert snapshot["uptime_s"] == 5.0
    assert snapshot["nats"] == {"connected": True, "reconnects": 0, "degraded": False}
    assert snapshot["lanes"]["audio"]["state"] == "not_provisioned"
    assert snapshot["lanes"]["audio"]["done"] == {
        "ok": 0,
        "empty": 0,
        "missing": 0,
        "rejected": 0,
        "failed": 0,
    }
    assert snapshot["slots"] == {"muq": {"state": "ready", "restarts": 0}}
    assert snapshot["outbox"] == {"pending": 0, "publish_failures": 0}
    counters = snapshot["counters"]
    assert counters["lease_lost"] == 1
    assert counters["align_engine"] == {"qwen": 0, "mms": 1, "gapfill": 0, "global": 0}
    assert counters["llm_calls"] == {"anthropic": {"ok": 1}}
    assert counters["llm_latency_ms_p95"] == {"anthropic": 120.0}
    assert counters["complete_answers"] == 0
    assert counters["complete_rejected"] == counters["rpc_handed_on"] == {}
    assert counters["rpc_expired_by_method"] == {}
    assert snapshot["lanes"]["audio"]["pull_unanswered"] == 0
    assert snapshot["gpu"]["name"] == "RTX 3060"
    json.dumps(snapshot)


async def test_snapshot_carries_the_complete_and_hand_over_counters(
    harness: Harness, settings: Settings
) -> None:
    harness.counters.inc("complete_answers_total", by=3)
    harness.counters.inc("complete_rejected_total", why="busy")
    harness.counters.inc("complete_rejected_total", why="off_schema", by=2)
    harness.counters.inc("complete_unavailable_total", state="off")
    harness.counters.inc("rpc_handed_on_total", method="complete", by=4)
    harness.counters.inc("rpc_expired_total", method="complete", by=2)
    harness.counters.inc("rpc_expired_total", method="match_track")
    harness.counters.inc("pull_unanswered_total", lane="audio")

    snapshot = reporter(harness, settings).snapshot()

    counters = snapshot["counters"]
    assert counters["complete_answers"] == 3
    assert counters["complete_rejected"] == {"busy": 1, "off_schema": 2}
    assert counters["complete_unavailable"] == {"off": 1}
    assert counters["rpc_handed_on"] == {"complete": 4}
    assert counters["rpc_expired"] == 3
    assert counters["rpc_expired_by_method"] == {"complete": 2, "match_track": 1}
    assert snapshot["lanes"]["audio"]["pull_unanswered"] == 1


async def test_status_is_published_every_fifteen_seconds(
    harness: Harness, settings: Settings
) -> None:
    status = reporter(harness, settings)
    loop = asyncio.create_task(status.run())
    await harness.settle()
    assert len(harness.bus.core_published) == 1
    await harness.clock.tick(STATUS_INTERVAL_S)
    assert len(harness.bus.core_published) == 2
    subject, body, _ = harness.bus.core_published[-1]
    assert subject == "worker.status.test-worker"
    assert json.loads(body)["worker_id"] == "test-worker"
    harness.bus.closed = True
    await harness.clock.tick(STATUS_INTERVAL_S)
    assert harness.counters.value("status_publish_failures_total") == 1
    loop.cancel()
    await asyncio.gather(loop, return_exceptions=True)


async def test_health_requests_are_answered_on_trusted_nodes(
    harness: Harness, settings: Settings
) -> None:
    status = reporter(harness, settings)
    assert await status.serve_health()
    [subscription] = harness.bus.subscriptions
    assert subscription.subject == "worker.health.test-worker"
    request = Msg(harness.bus, subject=subscription.subject, reply="_INBOX.ops.1", data=b"")
    await subscription.callback(request)
    subject, body, _ = harness.bus.core_published[-1]
    assert subject == "_INBOX.ops.1"
    lane = json.loads(body)["lanes"]["audio"]
    assert lane["inflight"] == 0
    assert (lane["per_hour"], lane["task_p50_ms"], lane["task_p95_ms"]) == (0.0, 0.0, 0.0)


async def test_public_nodes_do_not_subscribe_to_health(
    harness: Harness, settings: Settings
) -> None:
    public = dataclasses.replace(
        settings, worker=dataclasses.replace(settings.worker, trust="public")
    )
    assert not await reporter(harness, public).serve_health()
    assert harness.bus.subscriptions == []


def test_parse_labels() -> None:
    assert parse_labels("") == {}
    assert parse_labels("lane=audio,status=ok") == {"lane": "audio", "status": "ok"}
