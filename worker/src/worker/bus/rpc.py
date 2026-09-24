from __future__ import annotations

import asyncio
import json
import logging
import time
from collections.abc import Callable, Mapping
from typing import Protocol

import nats.errors
from nats.aio.msg import Msg

from worker.bus.connection import Connection
from worker.bus.lane_runner import Aborted
from worker.bus.lease import Lease, Leases, Settled
from worker.bus.outbox import encode
from worker.contract import Contract, LaneSpec
from worker.domain.deadline import Clock, Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.observability.counters import Counters

log = logging.getLogger("worker.bus.rpc")

REPLY_MARGIN_S = 0.5
PROCESS_MARGIN_S = 1.5
ERROR_EXPIRED = "expired"
ERROR_INVALID = "invalid_request"
ERROR_INTERNAL = "internal"


class RpcMethod(Protocol):
    async def __call__(
        self, request: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]: ...


class RpcHandler:
    def __init__(
        self,
        lane: LaneSpec,
        methods: Mapping[str, RpcMethod],
        leases: Leases,
        connection: Connection,
        contract: Contract,
        counters: Counters,
        clock: Clock,
        worker_id: str,
        build: str,
        wall: Callable[[], float] = time.time,
    ) -> None:
        self.lane = lane
        self._methods = methods
        self._leases = leases
        self._connection = connection
        self._contract = contract
        self._counters = counters
        self._clock = clock
        self._worker_id = worker_id
        self._build = build
        self._wall = wall
        self._prefix = lane.filter_subject.removesuffix(">")

    async def handle(self, msg: Msg, abort: asyncio.Event) -> None:
        method = msg.subject.removeprefix(self._prefix)
        seq = msg.metadata.sequence.stream
        attached = self._leases.attach(msg, {}, f"{self.lane.correlation_prefix}:{seq}")
        if attached.redelivered:
            return
        lease = attached.lease
        heartbeat = asyncio.create_task(lease.heartbeat())
        try:
            reply_to = (msg.headers or {}).get(self._contract.rpc.reply_header)
            if reply_to is None:
                self._counters.inc("rpc_no_reply_to_total")
                log.warning("rpc_no_reply_to", extra={**lease.log_fields(), "method": method})
            else:
                await self._reply(lease, method, reply_to, await self._answer(msg, method, abort))
            await lease.ack()
            lease.settle(Settled.PUBLISHED)
        finally:
            heartbeat.cancel()
            await asyncio.gather(heartbeat, return_exceptions=True)
            if not lease.is_settled:
                self._counters.inc("lease_unsettled_total", lane=self.lane.name)
                log.error("lease_left_unsettled", extra=lease.log_fields())
                lease.settle(Settled.DROPPED)

    async def _answer(self, msg: Msg, method: str, abort: asyncio.Event) -> dict[str, object]:
        deadline = self._deadline(msg, method)
        if deadline.remaining() - REPLY_MARGIN_S <= 0:
            self._counters.inc("rpc_expired_total")
            return error_reply(ERROR_EXPIRED)
        try:
            request = json.loads(msg.data)
        except ValueError:
            return error_reply(ERROR_INVALID)
        if not isinstance(request, dict):
            return error_reply(ERROR_INVALID)
        call = self._methods.get(method)
        if call is None:
            self._counters.inc("rpc_unknown_method_total", method=method)
            return error_reply(ERROR_INVALID)
        try:
            data = dict(await self._call(call, request, deadline.minus(PROCESS_MARGIN_S), abort))
        except Aborted:
            self._counters.inc("rpc_expired_total")
            return error_reply(ERROR_EXPIRED)
        except PermanentFailure as failure:
            if failure.reason is Reason.INVALID_REQUEST:
                return error_reply(ERROR_INVALID)
            self._counters.inc("rpc_internal_total", method=method)
            log.warning("rpc_permanent_failure", extra={"method": method, "error": str(failure)})
            return error_reply(ERROR_INTERNAL)
        except TransientFailure as failure:
            if failure.reason is Reason.DEADLINE_EXCEEDED:
                self._counters.inc("rpc_expired_total")
                return error_reply(ERROR_EXPIRED)
            self._counters.inc("rpc_internal_total", method=method)
            log.warning("rpc_transient_failure", extra={"method": method, "error": str(failure)})
            return error_reply(ERROR_INTERNAL)
        except Exception as error:
            self._counters.inc("rpc_internal_total", method=method)
            log.error("rpc_crashed", extra={"method": method}, exc_info=error)
            return error_reply(ERROR_INTERNAL)
        return {"ok": True, "data": data}

    async def _call(
        self,
        call: RpcMethod,
        request: Mapping[str, object],
        deadline: Deadline,
        abort: asyncio.Event,
    ) -> Mapping[str, object]:
        work = asyncio.ensure_future(call(request, deadline))
        stop = asyncio.ensure_future(abort.wait())
        try:
            done, _ = await asyncio.wait({work, stop}, return_when=asyncio.FIRST_COMPLETED)
        except asyncio.CancelledError:
            work.cancel()
            stop.cancel()
            await asyncio.gather(work, stop, return_exceptions=True)
            raise
        stop.cancel()
        await asyncio.gather(stop, return_exceptions=True)
        if work in done:
            return work.result()
        work.cancel()
        await asyncio.gather(work, return_exceptions=True)
        raise Aborted

    async def _reply(
        self, lease: Lease, method: str, reply_to: str, reply: Mapping[str, object]
    ) -> None:
        names = self._contract.headers
        headers = {
            names.worker_id: self._worker_id,
            names.worker_build: self._build,
            names.deliveries: str(lease.num_delivered),
        }
        reply, body = self._encode(lease, method, reply)
        try:
            await self._connection.client.publish(reply_to, body, headers=headers)
        except nats.errors.Error as error:
            self._counters.inc("rpc_reply_failures_total")
            log.error(
                "rpc_reply_failed",
                extra={**lease.log_fields(), "method": method, "error": str(error)},
            )
            return
        self._counters.inc("rpc_replies_total", method=method, ok=str(reply.get("ok")))
        log.info(
            "rpc_finished",
            extra={
                **lease.log_fields(),
                "method": method,
                "ok": reply.get("ok"),
                "error": reply.get("error"),
            },
        )

    def _encode(
        self, lease: Lease, method: str, reply: Mapping[str, object]
    ) -> tuple[Mapping[str, object], bytes]:
        try:
            return reply, encode(reply)
        except ValueError as error:
            self._counters.inc("rpc_internal_total", method=method)
            log.error(
                "rpc_reply_unencodable",
                extra={**lease.log_fields(), "method": method, "error": str(error)},
            )
            fallback = error_reply(ERROR_INTERNAL)
            return fallback, encode(fallback)

    def _deadline(self, msg: Msg, method: str) -> Deadline:
        header = (msg.headers or {}).get(self._contract.rpc.deadline_header)
        if header is not None:
            try:
                return Deadline.from_epoch_ms(int(header), self._wall, self._clock.now)
            except ValueError:
                self._counters.inc("rpc_bad_deadline_total")
                log.warning("rpc_bad_deadline", extra={"method": method, "header": header})
        window_s = self._contract.rpc.windows_s.get(method, self.lane.deadline_s)
        delivered_at = msg.metadata.timestamp.timestamp()
        return Deadline.from_epoch_ms(
            int((delivered_at + window_s) * 1000), self._wall, self._clock.now
        )


def error_reply(error: str) -> dict[str, object]:
    return {"ok": False, "error": error}
