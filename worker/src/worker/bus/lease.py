from __future__ import annotations

import asyncio
import logging
from collections.abc import Callable, Mapping
from contextvars import ContextVar
from dataclasses import dataclass
from enum import Enum

import nats.errors
from nats.aio.msg import Msg

from worker.bus.connection import Connection, race
from worker.contract import LaneSpec
from worker.domain.deadline import Clock, Deadline
from worker.domain.outcome import LeaseDropped, Status
from worker.observability.counters import Counters

log = logging.getLogger("worker.bus.lease")

ACK_TIMEOUT_S = 5.0
ACK_ATTEMPTS = 3
ACK_RETRY_PAUSE_S = 1.0


class Settled(Enum):
    PUBLISHED = "published"
    NACKED = "nacked"
    DROPPED = "dropped"


class LastDelivery(Exception):
    pass


current_lease: ContextVar[Lease | None] = ContextVar("current_lease", default=None)


def drop_if_stale() -> None:
    lease = current_lease.get()
    if lease is not None and lease.is_stale:
        raise LeaseDropped(f"lease of stream_seq {lease.stream_seq} is stale")


class Lease:
    def __init__(
        self,
        lane: LaneSpec,
        msg: Msg,
        payload: Mapping[str, object],
        correlation: str,
        connection: Connection,
        counters: Counters,
        clock: Clock,
        max_deliver: Callable[[], int],
        msg_id_header: str,
    ) -> None:
        self.lane = lane
        self.payload = payload
        self.correlation = correlation
        self.stream_seq = msg.metadata.sequence.stream
        self.msg_id = (msg.headers or {}).get(msg_id_header)
        self.deadline = Deadline.after(lane.deadline_s, clock.now)
        self.settled: asyncio.Future[Settled] = asyncio.get_running_loop().create_future()
        self._msg = msg
        self._connection = connection
        self._counters = counters
        self._clock = clock
        self._max_deliver = max_deliver
        self._last_progress_at = clock.now()
        self._stale = False

    @property
    def num_delivered(self) -> int:
        return self._msg.metadata.num_delivered

    @property
    def is_last_delivery(self) -> bool:
        return self.lane.is_last_delivery(self.num_delivered, self._max_deliver())

    @property
    def is_settled(self) -> bool:
        return self.settled.done()

    @property
    def is_stale(self) -> bool:
        if self._stale:
            return True
        window = self.lane.ack_wait_s - self.lane.heartbeat_s
        since = self._last_progress_at
        silent_s = self._clock.now() - since
        outage_s = self._connection.longest_outage_since(since)
        if silent_s <= window and outage_s <= window:
            return False
        self._stale = True
        self._counters.inc("lease_lost_total", lane=self.lane.name)
        log.warning(
            "lease_lost",
            extra={
                **self.log_fields(),
                "silent_s": round(silent_s, 3),
                "outage_s": round(outage_s, 3),
            },
        )
        return True

    def adopt(self, msg: Msg) -> None:
        self._msg = msg
        self._last_progress_at = self._clock.now()
        self._stale = False
        log.info("lease_readopted", extra=self.log_fields())

    def done_msg_id(self, status: Status) -> str:
        return self.lane.done_msg_id(self.correlation, self.stream_seq, status.value)

    def log_fields(self) -> dict[str, object]:
        return {
            "lane": self.lane.name,
            "correlation": self.correlation,
            "nats_msg_id": self.msg_id,
            "stream_seq": self.stream_seq,
            "num_delivered": self.num_delivered,
        }

    async def heartbeat(self) -> None:
        while not self.settled.done():
            await self._clock.sleep(self.lane.heartbeat_s)
            if self.settled.done() or self.is_stale:
                continue
            if not self._connection.is_connected or self._connection.suspicious:
                continue
            try:
                await self._msg.in_progress()
            except (nats.errors.Error, OSError) as error:
                self._counters.inc("heartbeat_failures_total", lane=self.lane.name)
                log.warning("heartbeat_failed", extra={**self.log_fields(), "error": str(error)})
                continue
            self._last_progress_at = self._clock.now()

    async def ack(self) -> bool:
        self._require_open()
        give_up_at = self._clock.now() + ACK_TIMEOUT_S * ACK_ATTEMPTS
        attempt = 0
        while attempt < ACK_ATTEMPTS:
            if not self._connection.is_connected:
                if not await self._reconnected_before(give_up_at):
                    break
                continue
            attempt += 1
            try:
                await self._msg.ack_sync(timeout=ACK_TIMEOUT_S)
            except nats.errors.TimeoutError as error:
                if self._connection.is_connected:
                    self._connection.suspect("ack_sync")
                self._note_ack_failure(attempt, error)
            except nats.errors.Error as error:
                self._note_ack_failure(attempt, error)
                if attempt < ACK_ATTEMPTS:
                    await self._clock.sleep(ACK_RETRY_PAUSE_S)
            else:
                self._connection.trust()
                return True
        log.error("ack_abandoned", extra=self.log_fields())
        return False

    async def release_transient(self, delay_s: float) -> None:
        self._require_open()
        connected = self._connection.is_connected
        if self.is_stale or self._connection.suspicious or not connected:
            self._counters.inc("stale_transient_total", lane=self.lane.name)
            log.info(
                "stale_transient",
                extra={
                    **self.log_fields(),
                    "suspicious": self._connection.suspicious,
                    "connected": connected,
                },
            )
            self.settle(Settled.DROPPED)
            return
        if self.is_last_delivery:
            raise LastDelivery(f"stream_seq {self.stream_seq} is on its last delivery")
        try:
            await self._msg.nak(delay_s)
        except nats.errors.Error as error:
            self._counters.inc("nak_failures_total", lane=self.lane.name)
            log.warning("nak_failed", extra={**self.log_fields(), "error": str(error)})
            self.settle(Settled.DROPPED)
            return
        self._counters.inc("naks_total", lane=self.lane.name)
        log.info("task_nacked", extra={**self.log_fields(), "delay_s": delay_s})
        self.settle(Settled.NACKED)

    def settle(self, settled: Settled) -> None:
        if not self.settled.done():
            self.settled.set_result(settled)

    async def wait_settled(self) -> Settled:
        return await asyncio.shield(self.settled)

    def _require_open(self) -> None:
        if self.settled.done():
            raise RuntimeError(f"lease of stream_seq {self.stream_seq} is already settled")

    async def _reconnected_before(self, give_up_at: float) -> bool:
        left_s = give_up_at - self._clock.now()
        if left_s <= 0 or self._connection.closed.is_set():
            return False
        await race(
            self._connection.wait_connected(),
            self._connection.closed.wait(),
            self._clock.sleep(left_s),
        )
        return self._connection.is_connected

    def _note_ack_failure(self, attempt: int, error: Exception) -> None:
        self._counters.inc("ack_failures_total", lane=self.lane.name)
        log.warning(
            "ack_failed",
            extra={**self.log_fields(), "attempt": attempt, "error": str(error)},
        )


@dataclass(frozen=True)
class Attached:
    lease: Lease
    redelivered: bool


class Leases:
    def __init__(
        self,
        lane: LaneSpec,
        connection: Connection,
        counters: Counters,
        clock: Clock,
        max_deliver: Callable[[], int],
        msg_id_header: str,
    ) -> None:
        self.lane = lane
        self._connection = connection
        self._counters = counters
        self._clock = clock
        self._max_deliver = max_deliver
        self._msg_id_header = msg_id_header
        self._active: dict[int, Lease] = {}

    def attach(self, msg: Msg, payload: Mapping[str, object], correlation: str) -> Attached:
        seq = msg.metadata.sequence.stream
        lease = self._active.get(seq)
        if lease is not None and not lease.is_settled:
            lease.adopt(msg)
            self._counters.inc("redelivered_to_owner_total", lane=self.lane.name)
            return Attached(lease, redelivered=True)
        lease = Lease(
            self.lane,
            msg,
            payload,
            correlation,
            self._connection,
            self._counters,
            self._clock,
            self._max_deliver,
            self._msg_id_header,
        )
        self._active[seq] = lease
        lease.settled.add_done_callback(lambda _: self._forget(seq, lease))
        return Attached(lease, redelivered=False)

    def get(self, stream_seq: int) -> Lease | None:
        return self._active.get(stream_seq)

    def __len__(self) -> int:
        return len(self._active)

    def _forget(self, seq: int, lease: Lease) -> None:
        if self._active.get(seq) is lease:
            del self._active[seq]
