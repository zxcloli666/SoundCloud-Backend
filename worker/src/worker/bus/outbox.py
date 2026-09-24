from __future__ import annotations

import asyncio
import logging
import math
from collections.abc import Mapping

import nats.errors
import nats.js.errors
import numpy as np
import orjson
from nats.js import JetStreamContext, api

from worker.bus.connection import Connection, race, sleep_unless
from worker.contract import HeaderNames
from worker.domain.deadline import Clock
from worker.observability.counters import Counters
from worker.settings import OutboxSettings

log = logging.getLogger("worker.bus.outbox")

PUBACK_TIMEOUT_S = 5.0
BACKOFF_START_S = 1.0
BACKOFF_CAP_S = 60.0
REJECTING_ERR_CODES = frozenset({10054, 10076})


class PublishRejected(Exception):
    pass


class PublishTimedOut(Exception):
    pass


class ConnectionClosed(PublishTimedOut):
    pass


def encode(payload: Mapping[str, object]) -> bytes:
    reject_non_finite(payload)
    try:
        return orjson.dumps(payload, option=orjson.OPT_SERIALIZE_NUMPY)
    except orjson.JSONEncodeError as error:
        raise ValueError(f"unencodable payload: {error}") from error


class Outbox:
    def __init__(
        self,
        js: JetStreamContext,
        connection: Connection,
        limits: OutboxSettings,
        counters: Counters,
        clock: Clock,
        worker_id: str,
        build: str,
        names: HeaderNames,
    ) -> None:
        self.pending = 0
        self.pending_bytes = 0
        self._js = js
        self._connection = connection
        self._max_results = limits.max_results
        self._max_bytes = limits.max_mib << 20
        self._counters = counters
        self._clock = clock
        self._worker_id = worker_id
        self._build = build
        self._names = names
        self._idle = asyncio.Event()
        self._idle.set()

    @property
    def has_room(self) -> bool:
        return self.pending < self._max_results and self.pending_bytes < self._max_bytes

    def headers(self, msg_id: str, deliveries: int) -> dict[str, str]:
        return {self._names.msg_id: msg_id, **self.worker_headers(deliveries)}

    def worker_headers(self, deliveries: int) -> dict[str, str]:
        return {
            self._names.worker_id: self._worker_id,
            self._names.worker_build: self._build,
            self._names.deliveries: str(deliveries),
        }

    async def publish(
        self,
        subject: str,
        body: bytes,
        headers: Mapping[str, str],
        lane: str,
        give_up_after_s: float | None,
    ) -> api.PubAck:
        self._take(len(body))
        try:
            return await self._publish_until_acked(subject, body, headers, lane, give_up_after_s)
        finally:
            self._give_back(len(body))

    async def flush(self, timeout_s: float) -> bool:
        await sleep_unless(self._clock, timeout_s, self._idle)
        return self._idle.is_set()

    async def _publish_until_acked(
        self,
        subject: str,
        body: bytes,
        headers: Mapping[str, str],
        lane: str,
        give_up_after_s: float | None,
    ) -> api.PubAck:
        started = self._clock.now()
        backoff_s = BACKOFF_START_S
        while True:
            await self._wait_connected(subject, started, give_up_after_s)
            try:
                ack = await self._js.publish(
                    subject, body, timeout=PUBACK_TIMEOUT_S, headers=dict(headers)
                )
            except nats.errors.MaxPayloadError as error:
                raise self._rejected(subject, headers, lane, error) from error
            except nats.js.errors.APIError as error:
                if is_permanent(error):
                    raise self._rejected(subject, headers, lane, error) from error
                self._note_failure(subject, headers, lane, error)
            except (nats.js.errors.NoStreamResponseError, nats.errors.Error) as error:
                if isinstance(error, nats.errors.TimeoutError) and self._connection.is_connected:
                    self._connection.suspect("puback")
                self._note_failure(subject, headers, lane, error)
            else:
                self._connection.trust()
                if ack.duplicate:
                    log.info("done_duplicate", extra={"subject": subject, "headers": dict(headers)})
                return ack
            self._check_give_up(subject, started, give_up_after_s)
            await self._clock.sleep(backoff_s)
            backoff_s = min(backoff_s * 2, BACKOFF_CAP_S)

    async def _wait_connected(
        self, subject: str, started: float, give_up_after_s: float | None
    ) -> None:
        while not self._connection.is_connected:
            self._check_give_up(subject, started, give_up_after_s)
            waits = [self._connection.wait_connected(), self._connection.closed.wait()]
            if give_up_after_s is not None:
                waits.append(self._clock.sleep(started + give_up_after_s - self._clock.now()))
            await race(*waits)

    def _check_give_up(self, subject: str, started: float, give_up_after_s: float | None) -> None:
        if self._connection.closed.is_set():
            raise ConnectionClosed(f"{subject}: connection closed")
        if give_up_after_s is not None and self._clock.now() - started >= give_up_after_s:
            raise PublishTimedOut(f"{subject}: no PubAck within {give_up_after_s} s")

    def _rejected(
        self, subject: str, headers: Mapping[str, str], lane: str, error: Exception
    ) -> PublishRejected:
        self._counters.inc("publish_rejected_total", lane=lane)
        log.error(
            "publish_rejected",
            extra={"subject": subject, "headers": dict(headers), "lane": lane, "error": str(error)},
        )
        return PublishRejected(f"{subject}: {error}")

    def _note_failure(
        self, subject: str, headers: Mapping[str, str], lane: str, error: Exception
    ) -> None:
        self._counters.inc("publish_failures_total", lane=lane)
        log.warning(
            "publish_failed",
            extra={"subject": subject, "headers": dict(headers), "lane": lane, "error": str(error)},
        )

    def _take(self, size: int) -> None:
        self.pending += 1
        self.pending_bytes += size
        self._idle.clear()
        self._counters.gauge("outbox_pending", self.pending)

    def _give_back(self, size: int) -> None:
        self.pending -= 1
        self.pending_bytes -= size
        if self.pending == 0:
            self._idle.set()
        self._counters.gauge("outbox_pending", self.pending)


def is_permanent(error: nats.js.errors.APIError) -> bool:
    return error.code == 400 or error.err_code in REJECTING_ERR_CODES


def reject_non_finite(value: object) -> None:
    if isinstance(value, float | np.floating):
        if not math.isfinite(value):
            raise ValueError(f"non-finite number {value}")
    elif isinstance(value, np.ndarray):
        if value.dtype.kind == "f" and not np.isfinite(value).all():
            raise ValueError(f"non-finite number in array of shape {value.shape}")
    elif isinstance(value, Mapping):
        for item in value.values():
            reject_non_finite(item)
    elif isinstance(value, list | tuple):
        for item in value:
            reject_non_finite(item)
