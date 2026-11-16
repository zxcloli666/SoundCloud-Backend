from __future__ import annotations

import json
import math
import time
import uuid
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

import jsonschema
import pytest
from nats.aio.subscription import Subscription

from tests.conftest import BASE_ENV, CONFIG_DIR
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
READ_SHARE = 0.5
NO_GRAMMAR: Mapping[str, object] = {
    "type": "object",
    "properties": {"x": {"$ref": "#/nowhere"}},
}
MARKED_HEAD = f"This conversation is about marked sections.\n{RULES}\nПерсонаж отвечает коротко.\n"
MARKED_TAILS = ("Now playing: M83.", "Now playing: Kino. Volume 40.")
TAIL_TOKENS = 64


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
        first = await answered(live, request("поставь громкость на 30"), window_s)
        started = time.monotonic()
        second = await ask(live, request("set the volume to 70 please"), window_s)
        warm_s = time.monotonic() - started
        refused = await ask_raw(live, {**request("hi"), "max_tokens": 0}, window_s)
        too_big = await ask(live, too_big_request(), window_s)
        no_grammar = await ask(live, {**request("hi"), "schema": NO_GRAMMAR}, window_s)
        not_an_object = await ask(live, {**request("hi"), "schema": {"type": "array"}}, window_s)
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
    for reply, level in ((first.reply, 30), (second, 70)):
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
    assert not_an_object == {"ok": False, "error": "invalid_request"}
    assert late == EXPIRED
    assert late_s < HURRIED_S
    assert after["ok"] is True
    assert {"name": "pause", "args": {}} in json.loads(after["data"]["text"])["actions"]
    assert off_schema == {"ok": False, "error": "internal"}
    assert hostile == [REFUSED] * len(hostile_bodies())
    assert all(call.reply["ok"] is True for call in alternating), alternating
    assert [call.tries for call in alternating[2:]] == [1, 1]
    assert capped_s < window_s
    assert capped["ok"] is True or capped == EXPIRED
    assert left == 0
    answers = [event for event in worker.events() if event.get("event") == "complete_answered"]
    reused = [event["reused_tokens"] > 0 for event in answers]
    cold = [first.tries > 1, True, True, *(call.tries > 1 for call in alternating[:2]), True, True]
    assert len(answers) == len(cold) + (capped["ok"] is True)
    assert reused[: len(cold)] == cold, [
        (event["input_tokens"], event["reused_tokens"]) for event in answers
    ]
    for call, event in zip([first, *alternating], [answers[0], *answers[3:7]], strict=True):
        assert call.tries <= tries_at_the_speed_of(event, call.seconds, window_s), (call, event)
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


@dataclass(frozen=True)
class Answered:
    reply: dict[str, object]
    tries: int
    seconds: float


async def answered(live: Live, payload: Mapping[str, object], window_s: float) -> Answered:
    tries = 0
    while True:
        tries += 1
        started = time.monotonic()
        reply = await ask(live, payload, window_s)
        if reply != EXPIRED or tries == COLD_TRIES:
            return Answered(reply, tries, time.monotonic() - started)


def tries_at_the_speed_of(event: Mapping[str, int], seconds: float, window_s: float) -> int:
    handled = event["input_tokens"] - event["reused_tokens"] + event["output_tokens"]
    read_per_try = handled / seconds * window_s * READ_SHARE
    return 1 + math.ceil(event["reused_tokens"] / read_per_try)


async def ask(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    assert live.contract.validate(METHOD, payload) == [], payload
    return await ask_raw(live, payload, window_s)


async def ask_raw(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    return await send(live, json.dumps(payload).encode(), window_s)


async def send(live: Live, body: bytes, window_s: float) -> dict[str, object]:
    inbox = f"_INBOX.e2e.{uuid.uuid4().hex}"
    subscription: Subscription = await live.admin.subscribe(inbox, max_msgs=1)
    headers = {
        "Nats-Msg-Id": f"rpc:{uuid.uuid4().hex}",
        "X-Reply-To": inbox,
        "X-Deadline": str(int((time.time() + window_s) * 1000)),
    }
    await live.js.publish(METHOD, body, headers=headers)
    answer = await subscription.next_msg(REPLY_TIMEOUT_S)
    return json.loads(answer.data)


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


async def test_a_new_tail_after_a_mark_of_the_system_is_all_that_its_first_call_reads(
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
        first = await answered(live, marked_request(MARKED_TAILS[0]), window_s)
        second = await ask(live, marked_request(MARKED_TAILS[1]), window_s)
        left = await live.messages_left("AI_RPC")
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    assert first.reply["ok"] is True and second["ok"] is True, (first, second)
    answers = [event for event in worker.events() if event.get("event") == "complete_answered"]
    assert len(answers) == 2
    assert (answers[0]["reused_tokens"] > 0) == (first.tries > 1)
    assert first.tries <= tries_at_the_speed_of(answers[0], first.seconds, window_s)
    assert 0 < answers[1]["input_tokens"] - answers[1]["reused_tokens"] < TAIL_TOKENS, answers
    assert left == 0
    assert_nothing_left(worker)


def marked_request(tail: str) -> dict[str, object]:
    return {
        **request("what is playing?"),
        "system": MARKED_HEAD + tail,
        "system_marks": [len(MARKED_HEAD.encode("utf-8"))],
    }
