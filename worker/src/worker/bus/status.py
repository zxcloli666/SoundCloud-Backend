from __future__ import annotations

import logging
from collections.abc import Callable, Mapping

import nats.errors
from nats.aio.msg import Msg

from worker.bus.connection import Connection
from worker.bus.lane_runner import LaneRunner
from worker.bus.outbox import Outbox, encode
from worker.contract import Contract
from worker.domain.deadline import Clock
from worker.observability.counters import Counters
from worker.settings import Settings

log = logging.getLogger("worker.bus.status")

STATUS_INTERVAL_S = 15.0
ALIGN_ENGINES = ("qwen", "mms", "gapfill", "global")

SlotsSnapshot = Callable[[], Mapping[str, Mapping[str, object]]]
GpuSnapshot = Callable[[], Mapping[str, object] | None]


class StatusReporter:
    def __init__(
        self,
        connection: Connection,
        contract: Contract,
        settings: Settings,
        counters: Counters,
        clock: Clock,
        lanes: Mapping[str, LaneRunner],
        outbox: Outbox,
        sync_version: str | None,
        slots: SlotsSnapshot,
        gpu: GpuSnapshot,
    ) -> None:
        self._connection = connection
        self._contract = contract
        self._settings = settings
        self._counters = counters
        self._clock = clock
        self._lanes = lanes
        self._outbox = outbox
        self._sync_version = sync_version
        self._slots = slots
        self._gpu = gpu
        self._started_at = clock.now()

    def snapshot(self) -> dict[str, object]:
        worker = self._settings.worker
        snapshot = self._counters.snapshot()
        counters = bucket_of(snapshot["counters"], "llm_calls_total")
        latency = bucket_of(snapshot["latency_ms"], "llm_latency_ms")
        return {
            "worker_id": worker.id,
            "build": worker.build,
            "profile": self._settings.profile,
            "trust": worker.trust,
            "sync_version": self._sync_version,
            "uptime_s": round(self._clock.now() - self._started_at, 3),
            "nats": {
                "connected": self._connection.is_connected,
                "reconnects": self._connection.reconnects,
                "degraded": self._connection.degraded,
            },
            "lanes": {name: runner.snapshot() for name, runner in self._lanes.items()},
            "slots": {name: dict(slot) for name, slot in self._slots().items()},
            "outbox": {
                "pending": self._outbox.pending,
                "publish_failures": self._counters.total("publish_failures_total"),
            },
            "counters": {
                "heartbeat_failures": self._counters.total("heartbeat_failures_total"),
                "lease_lost": self._counters.total("lease_lost_total"),
                "rpc_expired": self._counters.total("rpc_expired_total"),
                "separation_fallback": self._counters.total("separation_fallback_total"),
                "align_engine": {
                    engine: self._counters.value("align_engine_total", engine=engine)
                    for engine in ALIGN_ENGINES
                },
                "llm_calls": nested(counters, "provider", "outcome"),
                "llm_latency_ms_p95": p95_by(latency, "provider"),
            },
            "gpu": self._gpu(),
        }

    async def run(self) -> None:
        subject = self._contract.status_subject(self._settings.worker.id)
        while True:
            await self.publish(subject)
            await self._clock.sleep(STATUS_INTERVAL_S)

    async def publish(self, subject: str) -> bool:
        try:
            await self._connection.client.publish(subject, encode(self.snapshot()))
        except nats.errors.Error as error:
            self._counters.inc("status_publish_failures_total")
            log.warning("status_publish_failed", extra={"error": str(error)})
            return False
        return True

    async def serve_health(self) -> bool:
        if self._settings.is_public:
            return False
        subject = self._contract.health_subject(self._settings.worker.id)
        await self._connection.client.subscribe(subject, cb=self._answer_health)
        return True

    async def _answer_health(self, msg: Msg) -> None:
        try:
            await msg.respond(encode(self.snapshot()))
        except nats.errors.Error as error:
            self._counters.inc("health_reply_failures_total")
            log.warning("health_reply_failed", extra={"error": str(error)})


def bucket_of(section: Mapping[str, object], name: str) -> Mapping[str, object]:
    bucket = section.get(name)
    return bucket if isinstance(bucket, Mapping) else {}


def nested(bucket: Mapping[str, object], outer: str, inner: str) -> dict[str, dict[str, object]]:
    result: dict[str, dict[str, object]] = {}
    for key, value in bucket.items():
        labels = parse_labels(key)
        result.setdefault(labels.get(outer, ""), {})[labels.get(inner, "")] = value
    return result


def p95_by(bucket: Mapping[str, object], label: str) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in bucket.items():
        quantiles = value if isinstance(value, Mapping) else {}
        result[parse_labels(key).get(label, "")] = quantiles.get("p95", 0.0)
    return result


def parse_labels(key: str) -> dict[str, str]:
    if not key:
        return {}
    return dict(part.split("=", 1) for part in key.split(","))
