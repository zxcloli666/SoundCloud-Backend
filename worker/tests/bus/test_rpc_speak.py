from __future__ import annotations

import asyncio
import base64
import json
from collections.abc import Mapping

import pytest

from tests.bus.conftest import BUILD, WORKER_ID, Harness, build_harness
from tests.fakes.clock import FakeClock
from tests.fakes.encoder import FAKE_OGG, FakeEncoder
from tests.fakes.engines import FakeEngines
from tests.fakes.jetstream import FakeNats
from worker.bus.lease import Leases
from worker.bus.rpc import PROCESS_MARGIN_S, RpcHandler, RpcMethod
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import NotServedHere, Reason, TransientFailure
from worker.domain.ports import EngineUnavailable, Speech
from worker.domain.speak import ENCODE_MARGIN_S, Speaker
from worker.settings import Settings

LINE = {"voice": "loli", "text": "Ставлю следующий трек."}
VOICES = frozenset({"loli", "moriarty"})
WINDOW_S = 90
REPLY_TO = "speak.reply.1.3f2b9c4e-8a1d-4e6f-b2c7-5d9e0a1f4b63"


class GatedEngines(FakeEngines):
    def __init__(self) -> None:
        super().__init__()
        self.gate = asyncio.Event()
        self.gate.set()

    async def say(self, voice: str, text: str, deadline: Deadline) -> Speech:
        await self.gate.wait()
        return await super().say(voice, text, deadline)


class Speak:
    def __init__(self, harness: Harness, contract: Contract) -> None:
        self.harness = harness
        self.contract = contract
        self.engines = GatedEngines()
        self.encoder = FakeEncoder()
        speaker = Speaker(self.engines, self.encoder, contract, VOICES, harness.counters)
        self.handler = self.handler_for({"say": speaker.say})

    def handler_for(self, methods: Mapping[str, RpcMethod]) -> RpcHandler:
        harness = self.harness
        leases = Leases(
            harness.lane,
            harness.connection,
            harness.counters,
            harness.clock,
            lambda: harness.lane.max_deliver,
            self.contract.headers.msg_id,
        )
        return RpcHandler(
            harness.lane,
            methods,
            leases,
            harness.connection,
            self.contract,
            harness.counters,
            harness.clock,
            WORKER_ID,
            BUILD,
            wall=harness.clock.now,
        )

    def enqueue(self, payload: object = LINE, method: str = "say") -> int:
        deadline_ms = int((self.harness.clock.now() + WINDOW_S) * 1000)
        headers = {
            "Nats-Msg-Id": f"speak:{len(self.harness.bus.core_published)}",
            "X-Reply-To": REPLY_TO,
            "X-Deadline": str(deadline_ms),
        }
        return self.harness.bus.enqueue(f"speak.rpc.{method}", payload, headers)

    async def handle_one(self) -> asyncio.Task[None]:
        [msg] = await self.harness.fetch()
        return asyncio.create_task(self.handler.handle(msg, asyncio.Event()))

    async def run_one(self) -> dict[str, object] | None:
        await (await self.handle_one())
        return self.reply()

    def reply(self) -> dict[str, object] | None:
        replies = [entry for entry in self.harness.bus.core_published if entry[0] == REPLY_TO]
        return json.loads(replies[-1][1]) if replies else None

    def ack_kinds(self) -> list[str]:
        return [ack.kind for ack in self.harness.bus.acks()]


@pytest.fixture
async def speak(
    fake_nats: FakeNats, contract: Contract, settings: Settings, clock: FakeClock
) -> Speak:
    harness = await build_harness(fake_nats, contract, settings, clock, "speak", capacity=1)
    return Speak(harness, contract)


async def test_a_line_is_answered_with_its_clip_inside_the_reply(
    speak: Speak, contract: Contract
) -> None:
    speak.enqueue()

    reply = await speak.run_one()

    assert reply == {
        "ok": True,
        "data": {"seconds": 0.1, "clip": base64.b64encode(FAKE_OGG).decode()},
    }
    assert contract.validate("speak.rpc.say.reply", reply) == []
    assert speak.ack_kinds() == ["ack_sync"]
    [(_, call)] = speak.engines.calls
    assert call["remaining"] == pytest.approx(WINDOW_S - PROCESS_MARGIN_S - ENCODE_MARGIN_S)
    stores = speak.harness.bus.object_stores.values()
    assert all(store.puts == [] and store.gets == [] for store in stores)
    assert speak.harness.bus.published == []


async def test_the_reply_goes_to_the_reply_subject_the_api_named(speak: Speak) -> None:
    speak.enqueue()

    await speak.run_one()

    [(subject, body, headers)] = speak.harness.bus.core_published
    assert subject == REPLY_TO
    assert json.loads(body)["ok"] is True
    assert headers is not None and headers["X-Deliveries"] == "1"
    assert speak.harness.bus.published == []


async def test_a_slow_line_keeps_its_lease_with_heartbeats(speak: Speak) -> None:
    speak.engines.gate.clear()
    speak.enqueue()
    task = await speak.handle_one()
    await speak.harness.settle()
    lane = speak.harness.lane

    beats = int(lane.ack_wait_s // lane.heartbeat_s) + 2
    for _ in range(beats):
        await speak.harness.clock.tick(lane.heartbeat_s)
    speak.engines.gate.set()
    await task

    assert len(speak.harness.bus.acks("in_progress")) == beats
    assert speak.reply() is not None and speak.reply()["ok"] is True
    assert speak.harness.counters.value("lease_lost_total", lane="speak") == 0


@pytest.mark.parametrize(
    ("failure", "error"),
    [
        (EngineUnavailable("tts", "the engine is restarting"), "internal"),
        (TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=tts expired"), "expired"),
        (RuntimeError("the speech engine died"), "internal"),
    ],
    ids=["unavailable", "expired", "crashed"],
)
async def test_a_failed_line_is_answered_on_its_only_delivery(
    speak: Speak, failure: Exception, error: str
) -> None:
    speak.engines.fail("say", failure)
    speak.enqueue()

    assert await speak.run_one() == {"ok": False, "error": error}
    assert speak.ack_kinds() == ["ack_sync"]


async def test_a_line_no_worker_here_can_serve_is_never_handed_on(speak: Speak) -> None:
    async def not_here(request: Mapping[str, object], deadline: Deadline) -> Mapping[str, object]:
        raise NotServedHere(Reason.INTERNAL_ERROR, "no voice on this worker")

    speak.handler = speak.handler_for({"say": not_here})
    speak.enqueue()

    assert await speak.run_one() == {"ok": False, "error": "internal"}
    assert speak.ack_kinds() == ["ack_sync"]
    assert speak.harness.counters.total("rpc_handed_on_total") == 0


async def test_an_unknown_voice_is_an_invalid_request(speak: Speak) -> None:
    speak.enqueue({**LINE, "voice": "alice"})

    assert await speak.run_one() == {"ok": False, "error": "invalid_request"}
    assert speak.engines.calls == []


async def test_an_encoder_failure_is_internal(speak: Speak) -> None:
    speak.encoder.failure = TransientFailure(Reason.INTERNAL_ERROR, "ffmpeg rc=1")
    speak.enqueue()

    assert await speak.run_one() == {"ok": False, "error": "internal"}
    assert speak.harness.counters.value("speak_failed_total", why="encode") == 1


async def test_a_clip_over_its_schema_is_internal_and_never_sent(speak: Speak) -> None:
    speak.encoder.clip = b"OggS" + bytes(400_000)
    speak.enqueue()

    assert await speak.run_one() == {"ok": False, "error": "internal"}
    assert speak.harness.counters.value("speak_failed_total", why="reply") == 1


async def test_a_reply_over_the_lane_limit_is_internal_and_never_cut(speak: Speak) -> None:
    limit = speak.harness.lane.result_max_bytes

    async def huge(request: Mapping[str, object], deadline: Deadline) -> Mapping[str, object]:
        return {"seconds": 1.0, "clip": "A" * limit}

    speak.handler = speak.handler_for({"say": huge})
    speak.enqueue()

    assert await speak.run_one() == {"ok": False, "error": "internal"}
    assert all(len(entry[1]) <= limit for entry in speak.harness.bus.core_published)
    assert speak.harness.counters.value("rpc_reply_too_large_total", method="say") == 1
    assert speak.ack_kinds() == ["ack_sync"]
