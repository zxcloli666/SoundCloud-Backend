from __future__ import annotations

import asyncio
import json
import math
from collections.abc import Mapping, Sequence
from dataclasses import replace

import pytest

from tests.bus.conftest import BUILD, WORKER_ID, Harness, build_harness
from tests.conftest import BASE_ENV, CONFIG_DIR
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.jetstream import FakeNats
from tests.fakes.llm import FakeRefiner
from worker import settings as settings_module
from worker.bus.lane_runner import THROUGHPUT_WINDOW_S, LaneRunner
from worker.bus.lease import ACK_ATTEMPTS, ACK_RETRY_PAUSE_S, Leases
from worker.bus.rpc import PROCESS_MARGIN_S, REPLY_MARGIN_S, RpcHandler, RpcMethod
from worker.contract import Contract
from worker.domain.complete import Completer
from worker.domain.deadline import Deadline
from worker.domain.metadata.match import TrackMatcher
from worker.domain.metadata.resolve import ArtistResolver
from worker.domain.outcome import NotServedHere, PermanentFailure, Reason, TransientFailure
from worker.domain.ports import ChatMessage, Completion
from worker.settings import Settings

RESOLVE = {"title": "Artist - Song", "uploader": "chan", "metadata_artist": None}
COMPLETE = samples.RPC_REQUESTS["ai.rpc.complete"]
REFINED: Mapping[str, Mapping[str, object]] = {
    "resolve_artist": {"title": "Kendrick Lamar & SZA - luther", "uploader": "TDE"},
    "match_track": {
        "target": {"artist": "Billie Eilish", "title": "Ocean Eyes"},
        "candidates": [
            {"id": 1, "artist": "Billie Eilish", "title": "Ocean Eyes", "duration_sec": 200.0},
            {"id": 2, "artist": "billie", "title": "Ocean Eyes", "duration_sec": 201.0},
        ],
    },
    "complete": COMPLETE,
}
LOCAL_MODEL = "org/model-GGUF/model-Q4_K_M.gguf@0123abcd"
ANSWER = '{"say": "Kendrick Lamar, Luther", "actions": []}'
LOCAL = settings_module.load(CONFIG_DIR, BASE_ENV).llm.local
DEEP = 100_000
MAX_DELIVER = 5
CALLS = 3
SETTLE_TURNS = 100
REPLY_TO = "_INBOX.jobs.1"


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
        reply_to: str | None = REPLY_TO,
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
        replies = [entry for entry in self.harness.bus.core_published if entry[0] == REPLY_TO]
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
        lambda: MAX_DELIVER,
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
    assert rpc.harness.counters.value("rpc_expired_total", method="resolve_artist") == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


@pytest.mark.parametrize("method", ["resolve_artist", "match_track", "complete"])
async def test_domain_deadline_inside_the_process_margin_replies_expired(
    rpc: Rpc, contract: Contract, method: str
) -> None:
    resolver = ArtistResolver(None, rpc.harness.counters)
    matcher = TrackMatcher(None, rpc.harness.counters)
    engines = FakeEngines()
    completer = Completer(
        engines, contract.complete_limits, LOCAL, LOCAL_MODEL, rpc.harness.counters
    )
    methods = {
        "resolve_artist": resolver.resolve,
        "match_track": matcher.match,
        "complete": completer.complete,
    }
    rpc.handler = rpc_handler(rpc.harness, contract, methods)
    rpc.enqueue(
        samples.RPC_REQUESTS[f"ai.rpc.{method}"],
        method=method,
        deadline_ms=epoch_ms(rpc.harness.clock, PROCESS_MARGIN_S),
    )
    assert await rpc.run_one() == {"ok": False, "error": "expired"}
    assert engines.calls == []
    assert rpc.harness.counters.value("rpc_expired_total", method=method) == 1
    assert rpc.harness.counters.total("rpc_internal_total") == 0
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


class GatedEngines(FakeEngines):
    def __init__(self) -> None:
        super().__init__()
        self.gate = asyncio.Event()
        self.completion = replace(self.completion, text=ANSWER)

    async def complete(
        self,
        system: str,
        system_marks: Sequence[int],
        messages: Sequence[ChatMessage],
        schema: Mapping[str, object],
        max_tokens: int,
        deadline: Deadline,
    ) -> Completion:
        await self.gate.wait()
        return await super().complete(system, system_marks, messages, schema, max_tokens, deadline)


async def test_resolve_and_match_answer_while_complete_holds_the_local_model(
    rpc: Rpc, contract: Contract
) -> None:
    engines, chain = GatedEngines(), FakeRefiner()
    completer = Completer(
        engines, contract.complete_limits, LOCAL, LOCAL_MODEL, rpc.harness.counters
    )
    methods = {
        "resolve_artist": ArtistResolver(chain, rpc.harness.counters).resolve,
        "match_track": TrackMatcher(chain, rpc.harness.counters).match,
        "complete": completer.complete,
    }
    rpc.handler = rpc_handler(rpc.harness, contract, methods)
    handled = []
    for offset_s, method in enumerate(("complete", "resolve_artist", "match_track")):
        due = epoch_ms(rpc.harness.clock, 20 + offset_s)
        rpc.enqueue(REFINED[method], method=method, deadline_ms=due)
        [msg] = await rpc.harness.fetch()
        handled.append(asyncio.ensure_future(rpc.handler.handle(msg, asyncio.Event())))
    await asyncio.wait(handled[1:])
    early = len(rpc.harness.bus.acks("ack_sync"))
    engines.gate.set()
    await asyncio.gather(*handled)

    assert early == 2
    assert len(chain.calls) == 2
    assert [method for method, _ in engines.calls] == ["complete"]
    assert rpc.harness.counters.value("rpc_replies_total", method="complete", ok="True") == 1


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
    rpc.enqueue(payload=b"[" * DEEP + b"]" * DEEP)
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    rpc.enqueue(payload=[1, 2])
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    rpc.enqueue(method="match_track")
    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    assert rpc.harness.counters.value("rpc_unknown_method_total", method="match_track") == 1
    assert len(rpc.harness.bus.acks("ack_sync")) == 4
    assert rpc.harness.counters.value("lease_unsettled_total", lane="ai") == 0


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
    if error == "expired":
        assert rpc.harness.counters.value("rpc_expired_total", method="resolve_artist") == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


@pytest.mark.parametrize(
    ("reason", "error"),
    [(Reason.INTERNAL_ERROR, "internal"), (Reason.DEADLINE_EXCEEDED, "expired")],
    ids=["off", "busy"],
)
async def test_a_call_this_worker_cannot_serve_is_handed_on_and_answered_on_its_last_delivery(
    rpc: Rpc, reason: Reason, error: str
) -> None:
    rpc.failure = NotServedHere(reason, "the local model cannot take it")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    silent = [await rpc.run_one() for _ in range(MAX_DELIVER - 1)]

    assert silent == [None] * (MAX_DELIVER - 1)
    assert rpc.harness.counters.total("rpc_replies_total") == 0
    assert await rpc.run_one() == {"ok": False, "error": error}
    assert rpc.reply_headers()["X-Deliveries"] == str(MAX_DELIVER)
    assert [(ack.kind, ack.delay) for ack in rpc.harness.bus.acks()] == [
        *[("nak", None)] * (MAX_DELIVER - 1),
        ("ack_sync", None),
    ]
    assert rpc.harness.counters.value("rpc_handed_on_total", method="resolve_artist") == 4
    assert rpc.harness.counters.value("lease_unsettled_total", lane="ai") == 0


async def test_a_call_with_no_time_left_for_another_worker_is_answered_at_once(rpc: Rpc) -> None:
    rpc.failure = NotServedHere(Reason.INTERNAL_ERROR, "the local model is off on this worker")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, PROCESS_MARGIN_S + REPLY_MARGIN_S))

    assert await rpc.run_one() == {"ok": False, "error": "internal"}
    assert [ack.kind for ack in rpc.harness.bus.acks()] == ["ack_sync"]
    assert rpc.harness.counters.total("rpc_handed_on_total") == 0


async def test_a_call_is_answered_when_the_connection_cannot_be_trusted_with_a_hand_over(
    rpc: Rpc,
) -> None:
    rpc.failure = NotServedHere(Reason.INTERNAL_ERROR, "the local model is off on this worker")
    rpc.harness.connection.suspect("test")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))

    assert await rpc.run_one() == {"ok": False, "error": "internal"}
    assert [ack.kind for ack in rpc.harness.bus.acks()] == ["ack_sync"]
    assert rpc.harness.counters.total("rpc_handed_on_total") == 0


async def test_a_redelivery_that_overtakes_the_hand_over_is_handled_as_a_new_delivery(
    rpc: Rpc,
) -> None:
    rpc.failure = NotServedHere(Reason.INTERNAL_ERROR, "the local model is off on this worker")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    [msg] = await rpc.harness.fetch()
    send_nak = msg.nak

    async def nak_overtaken_by_the_redelivery(delay: float | None = None) -> None:
        await send_nak(delay)
        [again] = await rpc.harness.fetch()
        await rpc.handler.handle(again, asyncio.Event())

    msg.nak = nak_overtaken_by_the_redelivery
    await rpc.handler.handle(msg, asyncio.Event())

    assert [ack.num_delivered for ack in rpc.harness.bus.acks("nak")] == [1, 2]
    assert rpc.harness.counters.value("redelivered_to_owner_total", lane="ai") == 0


async def test_a_hand_over_that_cannot_be_sent_leaves_the_call_to_the_ack_wait(rpc: Rpc) -> None:
    rpc.failure = NotServedHere(Reason.INTERNAL_ERROR, "the local model is off on this worker")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    [msg] = await rpc.harness.fetch()
    rpc.harness.bus.closed = True
    handling = asyncio.create_task(rpc.handler.handle(msg, asyncio.Event()))
    await rpc.harness.settle()
    given_up = handling.done()
    handling.cancel()
    await asyncio.gather(handling, return_exceptions=True)

    assert given_up
    assert rpc.harness.counters.value("nak_failures_total", lane="ai") == 1
    assert rpc.harness.counters.value("naks_total", lane="ai") == 0
    assert rpc.harness.counters.value("rpc_reply_failures_total") == 0
    assert rpc.harness.counters.value("ack_failures_total", lane="ai") == 0
    assert rpc.harness.counters.value("lease_unsettled_total", lane="ai") == 0


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


class TimedEngines(FakeEngines):
    def __init__(self) -> None:
        super().__init__()
        self.remaining: list[float] = []
        self.completion = replace(
            self.completion, text=ANSWER, input_tokens=1500, reused_tokens=1320, output_tokens=18
        )

    async def complete(
        self,
        system: str,
        system_marks: Sequence[int],
        messages: Sequence[ChatMessage],
        schema: Mapping[str, object],
        max_tokens: int,
        deadline: Deadline,
    ) -> Completion:
        self.remaining.append(deadline.remaining())
        return await super().complete(system, system_marks, messages, schema, max_tokens, deadline)


def serve_complete(rpc: Rpc, contract: Contract) -> TimedEngines:
    engines = TimedEngines()
    completer = Completer(
        engines, contract.complete_limits, LOCAL, LOCAL_MODEL, rpc.harness.counters
    )
    rpc.handler = rpc_handler(rpc.harness, contract, {"complete": completer.complete})
    return engines


def local_worker(harness: Harness, contract: Contract, enabled: bool) -> LaneRunner:
    completer = Completer(
        TimedEngines(),
        contract.complete_limits,
        replace(LOCAL, enabled=enabled),
        LOCAL_MODEL,
        harness.counters,
    )
    handler = rpc_handler(harness, contract, {"complete": completer.complete})
    return LaneRunner(
        harness.lane,
        harness.watch,
        handler,
        harness.connection.js,
        harness.connection,
        harness.outbox,
        harness.runner.capacity,
        harness.counters,
        harness.clock,
    )


async def test_a_call_taken_by_a_worker_with_the_model_off_is_answered_by_one_that_has_it(
    rpc: Rpc, fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> None:
    without, having = rpc.harness, await build_harness(fake_nats, contract, settings, clock, "ai")
    runners = [local_worker(without, contract, False), local_worker(having, contract, True)]
    for harness, runner in zip((without, having), runners, strict=True):
        await harness.watch.check()
        harness.tasks.append(asyncio.create_task(runner.run()))
        await clock.tick()
    try:
        for _ in range(CALLS):
            rpc.enqueue(COMPLETE, method="complete", deadline_ms=epoch_ms(clock, 20))
            await clock.tick(turns=SETTLE_TURNS)
    finally:
        for harness, runner in zip((without, having), runners, strict=True):
            await harness.stop()
            await runner.cancel()
    replies = [entry for entry in fake_nats.core_published if entry[0] == REPLY_TO]

    assert [json.loads(body)["ok"] for _, body, _ in replies] == [True] * CALLS
    assert [headers["X-Deliveries"] for _, _, headers in replies] == ["2"] * CALLS
    assert without.counters.value("rpc_handed_on_total", method="complete") == CALLS
    assert without.counters.total("rpc_replies_total") == 0
    assert having.counters.value("rpc_replies_total", method="complete", ok="True") == CALLS


async def test_complete_replies_with_the_local_answer_inside_its_own_window(
    rpc: Rpc, contract: Contract
) -> None:
    engines = serve_complete(rpc, contract)
    seq = rpc.enqueue(COMPLETE, method="complete")

    reply = await rpc.run_one()

    assert reply == {
        "ok": True,
        "data": {"text": ANSWER, "model": LOCAL_MODEL, "input_tokens": 1500, "output_tokens": 18},
    }
    assert contract.validate("ai.rpc.complete.reply", reply) == []
    assert engines.remaining == [pytest.approx(30 - PROCESS_MARGIN_S)]
    [(method, asked)] = engines.calls
    assert method == "complete"
    assert (asked["system"], asked["schema"], asked["max_tokens"]) == (
        COMPLETE["system"],
        COMPLETE["schema"],
        COMPLETE["max_tokens"],
    )
    [ack] = rpc.harness.bus.acks()
    assert ack.kind == "ack_sync" and ack.seq == seq
    assert rpc.harness.counters.value("rpc_replies_total", method="complete", ok="True") == 1
    assert rpc.harness.counters.value("complete_reused_tokens_total") == 1320


async def test_complete_follows_the_deadline_the_caller_sent(rpc: Rpc, contract: Contract) -> None:
    engines = serve_complete(rpc, contract)
    rpc.enqueue(COMPLETE, method="complete", deadline_ms=epoch_ms(rpc.harness.clock, 12))

    reply = await rpc.run_one()

    assert reply is not None and reply["ok"] is True
    assert engines.remaining == [pytest.approx(12 - PROCESS_MARGIN_S)]


@pytest.mark.parametrize("header", [str(10**400), "9" * 30], ids=["overflowing", "far"])
async def test_a_caller_deadline_past_the_window_is_cut_to_the_window(
    rpc: Rpc, contract: Contract, header: str
) -> None:
    engines = serve_complete(rpc, contract)
    rpc.enqueue(COMPLETE, method="complete", deadline_ms=epoch_ms(rpc.harness.clock, 300))
    await rpc.run_one()
    rpc.enqueue(COMPLETE, method="complete", deadline_ms=0)
    [odd] = await rpc.harness.fetch()
    odd.headers["X-Deadline"] = header
    await rpc.handler.handle(odd, asyncio.Event())

    window_s = contract.rpc.windows_s["complete"]
    assert engines.remaining == [pytest.approx(window_s - PROCESS_MARGIN_S)] * 2


def nested(depth: int) -> dict[str, object]:
    schema: dict[str, object] = {"type": "object"}
    for _ in range(depth):
        schema = {"type": "object", "properties": {"x": schema}}
    return schema


@pytest.mark.parametrize(
    "change",
    [
        {"schema": nested(300)},
        {"schema": {"type": "integer", "maximum": 2**70}},
        {"schema": {"const": "\ud800"}},
        {"system": "rules \udfff"},
        {"messages": [{"role": "user", "content": "hi \ud800 there"}]},
    ],
    ids=["deep_schema", "wide_int", "surrogate_schema", "surrogate_system", "surrogate_message"],
)
async def test_hostile_complete_inputs_answer_invalid_request(
    rpc: Rpc, contract: Contract, change: Mapping[str, object]
) -> None:
    engines = serve_complete(rpc, contract)
    rpc.enqueue({**COMPLETE, **change}, method="complete")

    assert await rpc.run_one() == {"ok": False, "error": "invalid_request"}
    assert engines.calls == []
    assert rpc.harness.counters.value("complete_rejected_total", why="invalid_request") == 1
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"
    assert rpc.harness.counters.value("lease_unsettled_total", lane="ai") == 0


@pytest.mark.parametrize(
    ("change", "error"),
    [
        ({"max_tokens": 0}, "invalid_request"),
        ({"messages": []}, "invalid_request"),
        ({"schema": "object"}, "invalid_request"),
    ],
)
async def test_complete_refuses_a_request_outside_the_contract(
    rpc: Rpc, contract: Contract, change: Mapping[str, object], error: str
) -> None:
    engines = serve_complete(rpc, contract)
    rpc.enqueue({**COMPLETE, **change}, method="complete")

    assert await rpc.run_one() == {"ok": False, "error": error}
    assert engines.calls == []
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


@pytest.mark.parametrize(
    ("failure", "error"),
    [
        (TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=engine"), "expired"),
        (PermanentFailure(Reason.INVALID_REQUEST, "the prompt does not fit"), "invalid_request"),
        (PermanentFailure(Reason.MODEL_OUTPUT_INVALID, "bad reply"), "internal"),
    ],
)
async def test_complete_reports_what_the_local_model_could_not_do(
    rpc: Rpc, contract: Contract, failure: Exception, error: str
) -> None:
    engines = serve_complete(rpc, contract)
    engines.fail("complete", failure)
    rpc.enqueue(COMPLETE, method="complete")

    assert await rpc.run_one() == {"ok": False, "error": error}
    assert rpc.harness.bus.acks()[0].kind == "ack_sync"


async def test_complete_never_sends_an_answer_cut_at_the_token_limit(
    rpc: Rpc, contract: Contract
) -> None:
    engines = serve_complete(rpc, contract)
    engines.completion = replace(engines.completion, text='{"say": "Kendr', finished=False)
    rpc.enqueue(COMPLETE, method="complete")

    assert await rpc.run_one() == {"ok": False, "error": "internal"}
    assert rpc.harness.counters.value("complete_rejected_total", why="cut_at_max_tokens") == 1
    assert rpc.harness.counters.value("rpc_internal_total", method="complete") == 1


async def test_answered_calls_count_in_the_lane_throughput_and_handed_on_ones_do_not(
    rpc: Rpc,
) -> None:
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    await rpc.run_one()
    rpc.failure = NotServedHere(Reason.INTERNAL_ERROR, "the local model is off on this worker")
    rpc.enqueue(deadline_ms=epoch_ms(rpc.harness.clock, 20))
    await rpc.run_one()

    assert rpc.harness.counters.value("rpc_handed_on_total", method="resolve_artist") == 1
    throughput = rpc.harness.runner.throughput()
    assert throughput["per_hour"] == 3600 / THROUGHPUT_WINDOW_S
    assert rpc.harness.counters.latency("task_ms", lane="ai")["n"] == 1
