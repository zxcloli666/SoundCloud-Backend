from __future__ import annotations

import json
import time
import uuid
from collections.abc import Mapping
from pathlib import Path

import jsonschema
import pytest
from nats.aio.subscription import Subscription

from tests.conftest import BASE_ENV, CONFIG_DIR
from tests.contract import samples
from tests.integration.test_end_to_end import (
    Live,
    all_lanes_serving,
    assert_nothing_left,
    fixtures,
    live,
    start_serve,
    wait_for_status,
)
from tests.models.test_local_llm import ENGINE_SYSTEM_SENTENCES, RULES, SCHEMA, long_system
from worker import settings as s

pytestmark = [pytest.mark.integration, pytest.mark.local_llm]

__all__ = ["fixtures", "live"]

LANES = ("ai",)
METHOD = "ai.rpc.complete"
SHIPPED = s.load(CONFIG_DIR, BASE_ENV)
PATIENT_S = 300.0
HURRIED_S = 5.0
REPLY_TIMEOUT_S = 330.0
KILL_EVENTS = {"engine_call_deadline", "slot_call_failed", "rpc_crashed", "lease_left_unsettled"}
THREADS = "4"
STATE_CACHE_MIB = "80"
OFF_SCHEMA: Mapping[str, object] = {
    "type": "object",
    "properties": {"level": {"type": "number", "maximum": -1000}},
    "required": ["level"],
    "additionalProperties": False,
}
DEEP = 100_000
REFUSED = {"ok": False, "error": "invalid_request"}
EXPIRED = {"ok": False, "error": "expired"}
COLD_TRIES = 4
OFF_REPLY_S = 2.0


async def test_complete_is_served_by_the_local_model_over_nats(
    live: Live, fixtures: object, tmp_path: Path
) -> None:
    window_s = float(live.contract.rpc.windows_s["complete"])
    worker = await start_serve(
        live,
        fixtures,
        tmp_path,
        LANES,
        WORKER__RUNTIME__DEVICE="cpu",
        WORKER__LLM__LOCAL__THREADS=THREADS,
        WORKER__LLM__LOCAL__STATE_CACHE_MIB=STATE_CACHE_MIB,
    )
    try:
        await wait_for_status(worker, all_lanes_serving(LANES))
        first, first_tries = await answered(live, request("поставь громкость на 30"), window_s)
        started = time.monotonic()
        second = await ask(live, request("set the volume to 70 please"), window_s)
        warm_s = time.monotonic() - started
        refused = await ask_raw(live, {**request("hi"), "max_tokens": 0}, window_s)
        too_big = await ask(live, too_big_request(), window_s)
        no_grammar = await ask(live, {**request("hi"), "schema": {"type": "wat"}}, window_s)
        started = time.monotonic()
        late = await ask(live, hurried_request(), HURRIED_S)
        late_s = time.monotonic() - started
        after = await ask(live, request("pause the music"), window_s)
        off_schema = await ask(
            live, {**request("set the volume to 5"), "schema": OFF_SCHEMA}, window_s
        )
        hostile = [await send(live, body, window_s) for body in hostile_bodies()]
        alternating = [
            await answered(live, {**request(phrase), "system": system}, window_s)
            for system, phrase in alternating_turns()
        ]
        started = time.monotonic()
        capped = await ask(live, hurried_request(), PATIENT_S)
        capped_s = time.monotonic() - started
        left = await live.messages_left("AI_RPC")
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    for reply, level in ((first, 30), (second, 70)):
        assert live.contract.validate(f"{METHOD}.reply", reply) == [], reply
        assert reply["ok"] is True, reply
        data = reply["data"]
        answer = json.loads(data["text"])
        jsonschema.validate(answer, SCHEMA)
        assert {"name": "volume_set", "args": {"level": level}} in answer["actions"]
        assert data["model"] == s.local_llm_ref(SHIPPED)
        assert data["input_tokens"] > data["output_tokens"] > 0
    assert warm_s < window_s
    assert refused == {"ok": False, "error": "invalid_request"}
    assert too_big == {"ok": False, "error": "invalid_request"}
    assert no_grammar == {"ok": False, "error": "invalid_request"}
    assert late == EXPIRED
    assert late_s < HURRIED_S
    assert after["ok"] is True
    assert {"name": "pause", "args": {}} in json.loads(after["data"]["text"])["actions"]
    assert off_schema == {"ok": False, "error": "internal"}
    assert hostile == [REFUSED] * len(hostile_bodies())
    assert all(reply["ok"] is True for reply, _ in alternating), alternating
    assert [tries for _, tries in alternating[2:]] == [1, 1]
    assert capped_s < window_s
    assert capped["ok"] is True or capped == EXPIRED
    assert left == 0
    answers = [event for event in worker.events() if event.get("event") == "complete_answered"]
    reused = [event["reused_tokens"] > 0 for event in answers]
    cold = [first_tries > 1, True, True, *(tries > 1 for _, tries in alternating[:2]), True, True]
    assert reused == cold, [(event["input_tokens"], event["reused_tokens"]) for event in answers]
    assert all(event["input_tokens"] > event["output_tokens"] > 0 for event in answers)
    assert worker.event_names().isdisjoint(KILL_EVENTS), worker.tail()
    exits = [event for event in worker.events() if event.get("event") == "engine_exited"]
    assert {event["cause"] for event in exits} == {"stop"}
    assert_nothing_left(worker)


def request(phrase: str, max_tokens: int = 160) -> dict[str, object]:
    return {
        "system": RULES,
        "messages": [{"role": "user", "content": phrase}],
        "schema": SCHEMA,
        "max_tokens": max_tokens,
    }


def hurried_request() -> dict[str, object]:
    return {
        **request("pause the music"),
        "system": long_system("a caller in a hurry", ENGINE_SYSTEM_SENTENCES),
    }


async def answered(
    live: Live, payload: Mapping[str, object], window_s: float
) -> tuple[dict[str, object], int]:
    tries = 1
    reply = await ask(live, payload, window_s)
    while reply == EXPIRED and tries < COLD_TRIES:
        tries += 1
        reply = await ask(live, payload, window_s)
    return reply, tries


async def ask(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    assert live.contract.validate(METHOD, payload) == [], payload
    return await ask_raw(live, payload, window_s)


async def ask_raw(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    return await send(live, json.dumps(payload).encode(), window_s)


async def send(live: Live, body: bytes, window_s: float) -> dict[str, object]:
    inbox = f"_INBOX.e2e.{uuid.uuid4().hex}"
    subscription: Subscription = await live.admin.subscribe(inbox, max_msgs=1)
    headers = {
        "Nats-Msg-Id": f"rpc:complete:{uuid.uuid4().hex}",
        "X-Reply-To": inbox,
        "X-Deadline": str(int((time.time() + window_s) * 1000)),
    }
    await live.js.publish(METHOD, body, headers=headers)
    reply: dict[str, object] = json.loads((await subscription.next_msg(REPLY_TIMEOUT_S)).data)
    return reply


def too_big_request() -> dict[str, object]:
    return {**request("hi", max_tokens=64), "system": RULES + " la" * 9000}


def hostile_bodies() -> list[bytes]:
    nested: dict[str, object] = {"type": "object"}
    for _ in range(300):
        nested = {"type": "object", "properties": {"x": nested}}
    changes: list[Mapping[str, object]] = [
        {"schema": nested},
        {"schema": {"type": "integer", "maximum": 2**70}},
        {"system": "rules \udfff"},
        {"messages": [{"role": "user", "content": "hi \ud800"}]},
    ]
    bodies = [json.dumps({**request("hi"), **change}).encode() for change in changes]
    return [*bodies, b"[" * DEEP + b"]" * DEEP]


def alternating_turns() -> list[tuple[str, str]]:
    systems = [f"This conversation is about {topic}.\n{RULES}" for topic in ("mornings", "nights")]
    phrases = ["pause the music", "skip this track", "like this track", "set the volume to 40"]
    return [(systems[turn % 2], phrase) for turn, phrase in enumerate(phrases)]


async def test_a_worker_with_the_local_model_off_refuses_complete_at_once(
    live: Live, fixtures: object, tmp_path: Path
) -> None:
    window_s = float(live.contract.rpc.windows_s["complete"])
    worker = await start_serve(live, fixtures, tmp_path, LANES, WORKER__LLM__LOCAL__ENABLED="false")
    try:
        await wait_for_status(worker, all_lanes_serving(LANES))
        started = time.monotonic()
        off = await ask(live, request("pause the music"), window_s)
        off_s = time.monotonic() - started
        resolved = await live_call(live, "ai.rpc.resolve_artist", window_s)
        left = await live.messages_left("AI_RPC")
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    assert off == {"ok": False, "error": "internal"}
    assert off_s < OFF_REPLY_S
    assert resolved["ok"] is True
    assert left == 0
    spawned = [event for event in worker.events() if event.get("event") == "engine_spawned"]
    assert all(s.LOCAL_LLM_SLOT not in event["slots"] for event in spawned)
    assert any(
        event.get("event") == "local_llm_off" and event.get("why") == "config"
        for event in worker.events()
    )
    assert_nothing_left(worker)


async def live_call(live: Live, subject: str, window_s: float) -> dict[str, object]:
    payload = samples.RPC_REQUESTS[subject]
    assert live.contract.validate(subject, payload) == [], payload
    inbox = f"_INBOX.e2e.{uuid.uuid4().hex}"
    subscription: Subscription = await live.admin.subscribe(inbox, max_msgs=1)
    headers = {
        "Nats-Msg-Id": f"rpc:resolve:{uuid.uuid4().hex}",
        "X-Reply-To": inbox,
        "X-Deadline": str(int((time.time() + window_s) * 1000)),
    }
    await live.js.publish(subject, json.dumps(payload).encode(), headers=headers)
    reply: dict[str, object] = json.loads((await subscription.next_msg(REPLY_TIMEOUT_S)).data)
    return reply
