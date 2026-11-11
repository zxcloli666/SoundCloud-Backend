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
KILL_EVENTS = {"engine_call_deadline", "slot_call_failed"}


async def test_complete_is_served_by_the_local_model_over_nats(
    live: Live, fixtures: object, tmp_path: Path
) -> None:
    window_s = float(live.contract.rpc.windows_s["complete"])
    worker = await start_serve(live, fixtures, tmp_path, LANES, WORKER__RUNTIME__DEVICE="cpu")
    try:
        await wait_for_status(worker, all_lanes_serving(LANES))
        first = await ask(live, request("поставь громкость на 30"), PATIENT_S)
        started = time.monotonic()
        second = await ask(live, request("set the volume to 70 please"), window_s)
        warm_s = time.monotonic() - started
        refused = await ask_raw(live, {**request("hi"), "max_tokens": 0}, window_s)
        too_big = await ask(live, request("la " * 3900, max_tokens=64), window_s)
        no_grammar = await ask(live, {**request("hi"), "schema": {"type": "wat"}}, window_s)
        started = time.monotonic()
        late = await ask(live, hurried_request(), HURRIED_S)
        late_s = time.monotonic() - started
        after = await ask(live, request("pause the music"), PATIENT_S)
        left = await live.messages_left("AI_RPC")
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    for reply, level in ((first, 30), (second, 70)):
        assert live.contract.validate(f"{METHOD}.reply", reply) == [], reply
        assert reply["ok"] is True
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
    assert late == {"ok": False, "error": "expired"}
    assert late_s < HURRIED_S
    assert after["ok"] is True
    assert {"name": "pause", "args": {}} in json.loads(after["data"]["text"])["actions"]
    assert left == 0
    answered = [event for event in worker.events() if event.get("event") == "complete_answered"]
    assert [event["reused_tokens"] > 0 for event in answered] == [False, True, True]
    assert all(event["input_tokens"] > event["output_tokens"] > 0 for event in answered)
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


async def ask(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    assert live.contract.validate(METHOD, payload) == [], payload
    return await ask_raw(live, payload, window_s)


async def ask_raw(live: Live, payload: Mapping[str, object], window_s: float) -> dict[str, object]:
    inbox = f"_INBOX.e2e.{uuid.uuid4().hex}"
    subscription: Subscription = await live.admin.subscribe(inbox, max_msgs=1)
    headers = {
        "Nats-Msg-Id": f"rpc:complete:{uuid.uuid4().hex}",
        "X-Reply-To": inbox,
        "X-Deadline": str(int((time.time() + window_s) * 1000)),
    }
    await live.js.publish(METHOD, json.dumps(payload).encode(), headers=headers)
    reply: dict[str, object] = json.loads((await subscription.next_msg(REPLY_TIMEOUT_S)).data)
    return reply
