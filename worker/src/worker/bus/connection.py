from __future__ import annotations

import asyncio
import logging
import re
from collections import deque
from collections.abc import Awaitable
from typing import Any

import nats.errors
from nats.aio.client import Client
from nats.js import JetStreamContext

from worker.domain.deadline import Clock
from worker.observability.counters import Counters
from worker.settings import NatsSection

log = logging.getLogger("worker.bus.connection")

MAX_RECONNECT_ATTEMPTS = -1
RECONNECT_TIME_WAIT_S = 2
JETSTREAM_TIMEOUT_S = 5.0
FLUSH_TIMEOUT_S = 2
HISTORY = 64
DEGRADED_OUTAGE_S = 900.0
PROBE_INTERVAL_S = 5.0
PERMISSIONS_VIOLATION = re.compile(
    r'permissions violation for (publish|subscription) to "([^"]+)"', re.IGNORECASE
)


class Connection:
    def __init__(
        self,
        client: Client,
        settings: NatsSection,
        worker_id: str,
        counters: Counters,
        clock: Clock,
    ) -> None:
        self.client = client
        self.connected = asyncio.Event()
        self.closed = asyncio.Event()
        self.reconnects = 0
        self.disconnected_since: float | None = None
        self.suspicious = False
        self._settings = settings
        self._worker_id = worker_id
        self._counters = counters
        self._clock = clock
        self._outages: deque[tuple[float, float]] = deque(maxlen=HISTORY)
        self._denials: deque[tuple[str, float]] = deque(maxlen=HISTORY)
        self._js: JetStreamContext | None = None
        self._probe: asyncio.Task[None] | None = None
        self._closing = False

    async def open(self) -> None:
        await self.client.connect(
            servers=self._settings.url,
            user=self._settings.user,
            password=self._settings.password,
            name=self._worker_id,
            inbox_prefix=f"_INBOX.{self._worker_id}",
            ping_interval=int(self._settings.ping.interval_s),
            max_outstanding_pings=self._settings.ping.max_outstanding,
            max_reconnect_attempts=MAX_RECONNECT_ATTEMPTS,
            reconnect_time_wait=RECONNECT_TIME_WAIT_S,
            error_cb=self._on_error,
            disconnected_cb=self._on_disconnected,
            reconnected_cb=self._on_reconnected,
            closed_cb=self._on_closed,
        )
        self._js = self.client.jetstream(timeout=JETSTREAM_TIMEOUT_S)
        self.connected.set()
        log.info("nats_connected", extra={"worker_id": self._worker_id})

    async def close(self) -> None:
        self._closing = True
        await self._stop_probe()
        try:
            await self.client.flush(timeout=FLUSH_TIMEOUT_S)
        except (TimeoutError, nats.errors.Error) as error:
            self._counters.inc("nats_errors_total", kind="flush")
            log.warning("nats_flush_failed", extra={"error": str(error)})
        await self.client.close()
        self.connected.clear()
        self.closed.set()

    @property
    def js(self) -> JetStreamContext:
        if self._js is None:
            raise RuntimeError("connection is not open")
        return self._js

    @property
    def is_connected(self) -> bool:
        return bool(self.client.is_connected)

    async def wait_connected(self) -> None:
        await self.connected.wait()

    def outage_s(self) -> float:
        if self.disconnected_since is None:
            return 0.0
        return self._clock.now() - self.disconnected_since

    @property
    def degraded(self) -> bool:
        return self.outage_s() > DEGRADED_OUTAGE_S

    def longest_outage_since(self, since: float) -> float:
        longest = 0.0
        for started, ended in self._outages:
            if ended > since:
                longest = max(longest, ended - max(started, since))
        if self.disconnected_since is not None:
            longest = max(longest, self._clock.now() - max(self.disconnected_since, since))
        return longest

    def permission_denied(self, subject: str, since: float) -> bool:
        wanted = subject.lower()
        return any(denied == wanted and at >= since for denied, at in self._denials)

    def suspect(self, cause: str) -> None:
        if not self.suspicious:
            log.warning("nats_suspicious", extra={"cause": cause})
        self.suspicious = True
        if self._probe is None or self._probe.done():
            self._probe = asyncio.get_running_loop().create_task(self._probe_until_trusted())
            self._probe.add_done_callback(self._probe_finished)

    def trust(self) -> None:
        if self.suspicious:
            log.info("nats_trusted_again")
        self.suspicious = False

    async def _probe_until_trusted(self) -> None:
        while self.suspicious:
            await self.connected.wait()
            await self._clock.sleep(PROBE_INTERVAL_S)
            if not self.suspicious or not self.is_connected:
                continue
            try:
                await self.js.account_info()
            except (TimeoutError, nats.errors.Error) as error:
                self._counters.inc("nats_errors_total", kind="probe")
                log.warning("nats_probe_failed", extra={"error": str(error)})
                continue
            self.trust()

    def _probe_finished(self, task: asyncio.Task[None]) -> None:
        if task.cancelled():
            return
        error = task.exception()
        if error is not None:
            self._counters.inc("nats_errors_total", kind="probe_crashed")
            log.error("nats_probe_crashed", exc_info=error)

    async def _stop_probe(self) -> None:
        if self._probe is None:
            return
        self._probe.cancel()
        await asyncio.gather(self._probe, return_exceptions=True)
        self._probe = None

    async def _on_error(self, error: Exception) -> None:
        denied = PERMISSIONS_VIOLATION.search(str(error))
        if denied is not None:
            subject = denied.group(2).lower()
            self._denials.append((subject, self._clock.now()))
            self._counters.inc("nats_errors_total", kind="permissions")
            log.warning("nats_permission_denied", extra={"subject": subject})
            return
        self._counters.inc("nats_errors_total", kind=type(error).__name__)
        log.warning("nats_error", extra={"error": str(error)})

    async def _on_disconnected(self) -> None:
        if self.disconnected_since is None:
            self.disconnected_since = self._clock.now()
        self.connected.clear()
        self._counters.inc("nats_disconnects_total")
        log.warning("nats_disconnected")

    async def _on_reconnected(self) -> None:
        now = self._clock.now()
        if self.disconnected_since is not None:
            self._outages.append((self.disconnected_since, now))
        self.disconnected_since = None
        self.reconnects += 1
        self.connected.set()
        log.info("nats_reconnected", extra={"reconnects": self.reconnects})

    async def _on_closed(self) -> None:
        await self._stop_probe()
        self.connected.clear()
        self.closed.set()
        if self._closing:
            log.info("nats_closed")
            return
        log.error("nats_closed")


async def race(*waits: Awaitable[Any]) -> None:
    tasks = [asyncio.ensure_future(wait) for wait in waits]
    try:
        await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    finally:
        for task in tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


async def sleep_unless(clock: Clock, seconds: float, event: asyncio.Event) -> None:
    if event.is_set():
        return
    await race(clock.sleep(seconds), event.wait())
