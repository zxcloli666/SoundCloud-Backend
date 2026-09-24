from __future__ import annotations

import asyncio
import json
import math
from collections.abc import Mapping

import pytest

from tests.bus.conftest import BUILD, WORKER_ID, Harness, build_harness
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeNats
from worker.bus.lease import ACK_ATTEMPTS, ACK_RETRY_PAUSE_S, Leases
from worker.bus.rpc import PROCESS_MARGIN_S, REPLY_MARGIN_S, RpcHandler, RpcMethod
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.metadata.match import TrackMatcher
from worker.domain.metadata.resolve import ArtistResolver
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.settings import Settings

RESOLVE = {"title": "Artist - Song", "uploader": "chan", "metadata_artist": None}


class Rpc:
    def __init__(self, harness: Harness, handler: RpcHandler) -> None:
        self.harness = harness
        self.handler = handler
        self.calls: list[tuple[Mapping[str, object], float]] = []
        self.release = asyncio.Event()
        self.release.set()
        self.answer: Mapping[str, object] = {
            "primary_artist": "Artist",
            "featured": [],
            "producers": [],
            "remixers": [],
            "album": None,
            "confidence": 0.7,
            "source": "deterministic",
        }
        self.failure: Exception | None = None

    async def resolve(
        self, request: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        self.calls.append((request, deadline.remaining()))
        await self.release.wait()
        if self.failure is not None:
            raise self.failure
        return self.answer

    def enqueue(
        self,
        payload: object = RESOLVE,
        method: str = "resolve_artist",
        reply_to: str | None = "_INBOX.jobs.1",
        deadline_ms: int | None = None,
    ) -> int:
        headers = {"Nats-Msg-Id": f"rpc:{len(self.harness.bus.core_published)}:{deadline_ms}"}
        if reply_to is not None:
            headers["X-Reply-To"] = reply_to
        if deadline_ms is not None:
            headers["X-Deadline"] = str(deadline_ms)
        return self.harness.bus.enqueue(f"ai.rpc.{method}", payload, headers)

    async def run_one(self) -> dict[str, object] | None:
        [msg] = await self.harness.fetch()
        await self.handler.handle(msg, asyncio.Event())
        replies = [
            entry for entry in self.harness.bus.core_published if entry[0].startswith("_INBOX")
        ]
        return json.loads(replies[-1][1]) if replies else None

    def reply_headers(self) -> dict[str, str] | None:
        return self.harness.bus.core_published[-1][2]


def epoch_ms(clock: FakeClock, offset_s: float) -> int:
    return int((clock.now() + offset_s) * 1000)


@pytest.fixture
async def rpc(fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock) -> Rpc:
    harness = await build_harness(fake_nats, contract, settings, clock, "ai", capacity=64)
    stub = Rpc(harness, None)
    stub.handler = rpc_handler(harness, contract, {"resolve_artist": stub.resolve})
    return stub


def rpc_handler(
    harness: Harness, contract: Contract, methods: Mapping[str, RpcMethod]
) -> RpcHandler:
    clock = harness.clock
    leases = Leases(
        harness.lane,
        harness.connection,
        harness.counters,
        clock,
        lambda: 5,
        contract.headers.msg_id,
    )
    return RpcHandler(
        harness.lane,
        methods,
        leases,
        harness.connection,
        contract,
        harness.counters,
        clock,
        WORKER_ID,
        BUILD,
        wall=clock.now,
    )


async def test_reply_ok_then_ack_with_diagnostic_headers(rpc: Rpc, contract: Contract) -> None:
    seq = rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    reply = await rpc.run_one()
    assert reply == {"ok": True, "data": dict(rpc.answer)}
    assert contract.validate("ai.rpc.resolve_artist.reply", reply) == []
    assert rpc.reply_headers() == {
        "X-Worker-Id": WORKER_ID,
        "X-Worker-Build": BUILD,
        "X-Deliveries": "1",
    }
    [ack] = rpc.harness.bus.acks()
    assert ack.kind == "ack_sync" and ack.seq == seq
    [(request, remaining)] = rpc.calls
    assert request == RESOLVE
    assert remaining == pytest.approx(20 - PROCESS_MARGIN_S)


async def test_expired_deadline_replies_expired_and_acks(rpc: Rpc) -> None:
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, REPLY_MARGIN_S))
    assert await rpc.run_one() == {"ok": False, "error": "expired"}
    assert rpc.calls == []
    assert rpc.harness.counters.value("rpc_expired_total") == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


@pytest.mark.parametrize("method", ["resolve_artist", "match_track"])
async def test_domain_deadline_inside_the_process_margin_replies_expired(
    rpc: Rpc, contract: Contract, method: str
) -> None:
    resolver = ArtistResolver(None, rpc.harness.counters)
    matcher = TrackMatcher(None, rpc.harness.counters)
    methods = {"resolve_artist": resolver.resolve, "match_track": matcher.match}
    rpc.handler = rpc_handler(rpc.harness, contract, methods)
    rpc.enqueue(
        samples.RPC_REQUESTS[f"ai.rpc.{method}"],
        method=method,
        deadline_ms=epoch_ms(rpc.harness.clock, PROCESS_MARGIN_S),
    )
    assert await rpc.run_one() == {"ok": False, "error": "expired"}
    assert rpc.harness.counters.value("rpc_expired_total") == 1
    assert rpc.harness.counters.total("rpc_internal_total") == 0
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


async def test_missing_reply_to_acks_and_counts(rpc: Rpc) -> None:
    rpc.enqueue(reply_to=None)
    assert await rpc.run_one() is None
    assert rpc.harness.counters.value("rpc_no_reply_to_total") == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"
    assert rpc.calls == []


async def test_deadline_defaults_to_delivery_time_plus_window(rpc: Rpc, contract: Contract) -> None:
    rpc.enqueue()
    await rpc.run_one()
    [(_, remaining)] = rpc.calls
    assert remaining == pytest.approx(contract.rpc.windows_s["resolve_artist"] - PROCESS_MARGIN_S)
    rpc.enqueue(deadline_ms=None)
    rpc.harness.bus.streams["AI_RPC"].msg_ids.clear()
    [msg] = await rpc.harness.fetch()
    msg.headers["X-Deadline"] = "soon"
    await rpc.handler.handle(msg, asyncio.Event())
    assert rpc.harness.counters.value("rpc_bad_deadline_total") == 1


async def test_invalid_json_and_unknown_method_reply_invalid_request(rpc: Rpc) -> None:
    rpc.enqueue(payload=b"{oops")
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    rpc.enqueue(payload=[1, 2])
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    rpc.enqueue(method="match_track")
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    assert rpc.harness.counters.value("rpc_unknown_method_total", method="match_track") == 1
    assert len(rpc.harness.bus.acks("ack_sync")) == 3


@pytest.mark.parametrize(
    ("failure", "error", "counter"),
    [
        (PermanentFailure(Reason.INVALID_REQUEST, "schema"), "invalid_request", None),
        (PermanentFailure(Reason.MODEL_OUTPUT_INVALID), "internal", "rpc_internal_total"),
        (TransientFailure(Reason.DEADLINE_EXCEEDED), "expired", "rpc_expired_total"),
        (TransientFailure(Reason.INTERNAL_ERROR), "internal", "rpc_internal_total"),
        (KeyError("bug"), "internal", "rpc_internal_total"),
    ],
)
async def test_failures_map_to_the_error_envelope(
    rpc: Rpc, failure: Exception, error: str, counter: str | None
) -> None:
    rpc.failure = failure
    rpc.enqueue()
    assert await rpc.run_one() == {"ok": False, "error": error}
    if counter is not None:
        assert rpc.harness.counters.total(counter) == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


async def test_abort_during_shutdown_replies_expired(rpc: Rpc) -> None:
    rpc.release.clear()
    rpc.enqueue()
    [msg] = await rpc.harness.fetch()
    abort = asyncio.Event()
    task = asyncio.create_task(rpc.handler.handle(msg, abort))
    await rpc.harness.settle()
    assert rpc.calls
    abort.set()
    await task
    assert json.loads(rpc.harness.bus.core_published[-1][1]) == {"ok": False, "error": "expired"}
    assert rpc.harness.bus.acks()[-1].kind == "ack_sync"


async def test_redelivery_to_owner_is_adopted(rpc: Rpc) -> None:
    rpc.release.clear()
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 60))
    [msg] = await rpc.harness.fetch()
    task = asyncio.create_task(rpc.handler.handle(msg, asyncio.Event()))
    await rpc.harness.settle()
    await rpc.harness.clock.tick(31)
    [again] = await rpc.harness.fetch()
    await rpc.handler.handle(again, asyncio.Event())
    rpc.release.set()
    await task
    assert len(rpc.calls) == 1
    assert rpc.reply_headers()["X-Deliveries"] == "2"
    assert len(rpc.harness.bus.acks("ack_sync")) == 1


async def test_reply_failure_is_counted_but_still_acks(rpc: Rpc) -> None:
    rpc.enqueue()
    [msg] = await rpc.harness.fetch()
    rpc.harness.bus.closed = True
    handling = asyncio.create_task(rpc.handler.handle(msg, asyncio.Event()))
    await rpc.harness.settle()
    for _ in range(ACK_ATTEMPTS - 1):
        await rpc.harness.clock.tick(ACK_RETRY_PAUSE_S)
    await handling
    assert rpc.harness.counters.value("rpc_reply_failures_total") == 1
    assert rpc.harness.counters.value("ack_failures_total", lane="ai") == 3


@pytest.mark.parametrize(
    "answer",
    [
        {"primary_artist": "Artist", "confidence": math.nan},
        {"primary_artist": "Artist", "confidence": object()},
        ["not", "a", "mapping"],
    ],
)
async def test_unencodable_or_malformed_answer_replies_internal_and_acks(
    rpc: Rpc, answer: object
) -> None:
    rpc.answer = answer
    rpc.enqueue()
    assert await rpc.run_one() == {"ok": False, "error": "internal"}
    assert rpc.harness.counters.value("rpc_internal_total", method="resolve_artist") == 1
    assert len(rpc.harness.bus.acks("ack_sync")) == 1
    assert rpc.harness.counters.value("lease_unsettled_total", lane="ai") == 0
