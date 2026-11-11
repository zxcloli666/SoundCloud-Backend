from __future__ import annotations

import asyncio
from collections.abc import Mapping, Sequence
from dataclasses import replace

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from tests.contract import samples
from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from worker import settings as settings_module
from worker.contract import Contract
from worker.domain.complete import Completer
from worker.domain.deadline import Deadline
from worker.domain.outcome import NotServedHere, PermanentFailure, Reason, TransientFailure
from worker.domain.ports import ChatMessage, Completion, EngineUnavailable
from worker.observability.counters import Counters

METHOD = "ai.rpc.complete"
MODEL = "org/model-GGUF/model-Q4_K_M.gguf@0123abcd"
ANSWER = '{"say": "Готово", "actions": []}'
LOCAL = settings_module.load(CONFIG_DIR, BASE_ENV).llm.local


def request(**overrides: object) -> dict[str, object]:
    return {**samples.RPC_REQUESTS[METHOD], **overrides}


def completer(contract: Contract) -> tuple[Completer, FakeEngines, Counters]:
    engines, counters = FakeEngines(), Counters()
    engines.completion = replace(
        engines.completion, text=ANSWER, input_tokens=41, reused_tokens=30, output_tokens=9
    )
    return Completer(engines, contract.complete_limits, LOCAL, MODEL, counters), engines, counters


def deadline(clock: FakeClock, seconds: float = 30.0) -> Deadline:
    return Deadline(clock.now() + seconds, clock.now)


async def test_the_answer_carries_the_text_the_model_and_the_token_counts(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, counters = completer(contract)

    data = await subject.complete(request(), deadline(clock))

    assert data == {"text": ANSWER, "model": MODEL, "input_tokens": 41, "output_tokens": 9}
    assert contract.validate(f"{METHOD}.reply", {"ok": True, "data": dict(data)}) == []
    sent = samples.RPC_REQUESTS[METHOD]
    assert engines.calls == [
        (
            "complete",
            {
                "system": sent["system"],
                "system_marks": [],
                "messages": [
                    ChatMessage("user", "сделай потише"),
                    ChatMessage("assistant", '{"say": "Готово", "actions": []}'),
                    ChatMessage("user", "and what is playing now?"),
                ],
                "schema": sent["schema"],
                "max_tokens": 512,
            },
        )
    ]
    assert counters.value("complete_answers_total") == 1
    assert counters.value("complete_input_tokens_total") == 41
    assert counters.value("complete_reused_tokens_total") == 30
    assert counters.value("complete_output_tokens_total") == 9


LIMIT_CASES: Mapping[str, tuple[Mapping[str, object], bool]] = {
    "the sample": ({}, True),
    "an empty system": ({"system": ""}, True),
    "a system at the byte limit": ({"system": "я" * 24_000}, True),
    "a system over the byte limit": ({"system": "я" * 24_000 + "a"}, False),
    "a system that is not text": ({"system": None}, False),
    "no messages": ({"messages": []}, False),
    "messages that are not a list": ({"messages": {"role": "user", "content": "hi"}}, False),
    "the most messages": ({"messages": [{"role": "user", "content": "hi"}] * 64}, True),
    "too many messages": ({"messages": [{"role": "user", "content": "hi"}] * 65}, False),
    "a message that is not an object": ({"messages": ["hi"]}, False),
    "a system role in messages": ({"messages": [{"role": "system", "content": "hi"}]}, False),
    "a message without a role": ({"messages": [{"content": "hi"}]}, False),
    "an empty message": ({"messages": [{"role": "user", "content": ""}]}, False),
    "a message at the byte limit": ({"messages": [{"role": "user", "content": "я" * 6_000}]}, True),
    "a message over the byte limit": (
        {"messages": [{"role": "user", "content": "я" * 6_000 + "a"}]},
        False,
    ),
    "a history that ends with the assistant": (
        {"messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "{}"}]},
        True,
    ),
    "no marks listed": ({"system_marks": []}, True),
    "marks at the ends of two sections": (
        {"system": "Rules.\nPersona.\nState.", "system_marks": [7, 16]},
        True,
    ),
    "the most marks": ({"system": "abcdefgh", "system_marks": [1, 2, 3, 4]}, True),
    "too many marks": ({"system": "abcdefgh", "system_marks": [1, 2, 3, 4, 5]}, False),
    "a mark at the start": ({"system_marks": [0]}, False),
    "a mark past the byte limit": ({"system_marks": [48_001]}, False),
    "marks that are not a list": ({"system_marks": 7}, False),
    "marks given as null": ({"system_marks": None}, False),
    "a fractional mark": ({"system_marks": [1.5]}, False),
    "a boolean mark": ({"system_marks": [True]}, False),
    "a schema that is not an object": ({"schema": "object"}, False),
    "no schema": ({"schema": None}, False),
    "one answer token": ({"max_tokens": 1}, True),
    "the most answer tokens": ({"max_tokens": 1024}, True),
    "no answer tokens": ({"max_tokens": 0}, False),
    "too many answer tokens": ({"max_tokens": 1025}, False),
    "a boolean for answer tokens": ({"max_tokens": True}, False),
    "a fractional number of answer tokens": ({"max_tokens": 12.5}, False),
}


@pytest.mark.parametrize("case", sorted(LIMIT_CASES))
async def test_the_request_is_checked_exactly_as_the_contract_schema_says(
    contract: Contract, clock: FakeClock, case: str
) -> None:
    overrides, accepted = LIMIT_CASES[case]
    subject, engines, _ = completer(contract)
    asked = request(**overrides)

    assert (contract.validate(METHOD, asked) == []) is accepted
    if accepted:
        await subject.complete(asked, deadline(clock))
        assert len(engines.calls) == 1
        return
    with pytest.raises(PermanentFailure) as refused:
        await subject.complete(asked, deadline(clock))
    assert refused.value.reason is Reason.INVALID_REQUEST
    assert engines.calls == []


@pytest.mark.parametrize(
    ("system", "marks"),
    [("Rules.\nPersona.", [7, 15]), ("яя", [2]), ("я\nя", [2, 3, 5])],
    ids=["the last mark at the end", "between two-byte letters", "around a line break"],
)
async def test_marks_between_characters_of_the_system_reach_the_model_as_sent(
    contract: Contract, clock: FakeClock, system: str, marks: list[int]
) -> None:
    subject, engines, _ = completer(contract)

    await subject.complete(request(system=system, system_marks=marks), deadline(clock))

    [(_, asked)] = engines.calls
    assert (asked["system"], asked["system_marks"]) == (system, marks)


@pytest.mark.parametrize(
    ("system", "marks"),
    [("Rules.\nPersona.", [16]), ("abcdef", [4, 2]), ("abcdef", [3, 3]), ("я!", [1]), ("", [1])],
    ids=["past the end", "descending", "repeated", "inside a letter", "in an empty system"],
)
async def test_marks_that_do_not_fit_the_system_text_are_refused(
    contract: Contract, clock: FakeClock, system: str, marks: list[int]
) -> None:
    subject, engines, counters = completer(contract)
    asked = request(system=system, system_marks=marks)

    assert contract.validate(METHOD, asked) == []
    with pytest.raises(PermanentFailure) as refused:
        await subject.complete(asked, deadline(clock))
    assert refused.value.reason is Reason.INVALID_REQUEST
    assert engines.calls == []
    assert counters.value("complete_rejected_total", why="invalid_request") == 1


async def test_a_schema_over_the_exported_byte_limit_is_refused(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, _ = completer(contract)
    limit = contract.complete_limits.schema_bytes
    fits = {
        "type": "object",
        "description": "d" * (limit - len('{"type":"object","description":""}')),
    }
    over = {**fits, "description": str(fits["description"]) + "d"}

    await subject.complete(request(schema=fits), deadline(clock))
    with pytest.raises(PermanentFailure) as refused:
        await subject.complete(request(schema=over), deadline(clock))

    assert refused.value.reason is Reason.INVALID_REQUEST
    assert len(engines.calls) == 1


async def test_a_refusal_never_quotes_the_phrase(contract: Contract, clock: FakeClock) -> None:
    subject, _, _ = completer(contract)
    phrase = "секретная фраза пользователя"

    with pytest.raises(PermanentFailure) as refused:
        await subject.complete(
            request(messages=[{"role": "narrator", "content": phrase}], system=phrase),
            deadline(clock),
        )

    assert phrase not in str(refused.value)


async def test_a_request_without_time_left_never_reaches_the_model(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, _ = completer(contract)

    with pytest.raises(TransientFailure) as late:
        await subject.complete(request(), deadline(clock, seconds=0.4))

    assert late.value.reason is Reason.DEADLINE_EXCEEDED
    assert not isinstance(late.value, NotServedHere)
    assert engines.calls == []


async def test_an_expired_model_call_stays_expired(contract: Contract, clock: FakeClock) -> None:
    subject, engines, counters = completer(contract)
    engines.fail("complete", TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=complete"))

    with pytest.raises(TransientFailure) as late:
        await subject.complete(request(), deadline(clock))

    assert late.value.reason is Reason.DEADLINE_EXCEEDED
    assert not isinstance(late.value, NotServedHere)
    assert counters.value("complete_answers_total") == 0


async def test_an_unavailable_model_leaves_the_call_to_another_worker_as_an_internal_error(
    contract: Contract, clock: FakeClock
) -> None:
    subject, engines, counters = completer(contract)
    engines.fail("complete", EngineUnavailable("llm-local", "broken"))

    with pytest.raises(NotServedHere) as failed:
        await subject.complete(request(), deadline(clock))

    assert failed.value.reason is Reason.INTERNAL_ERROR
    assert counters.value("complete_unavailable_total", state="broken") == 1


@pytest.mark.parametrize(
    ("text", "finished", "why"),
    [
        ('{"say": "Гото', False, "cut_at_max_tokens"),
        ('{"say": "Готово"}', False, "cut_at_max_tokens"),
        ('["say"]', True, "not_a_json_object"),
        ("Готово", True, "not_a_json_object"),
        ('{"say": "Готово"}', True, "off_schema"),
        ('{"say": 7, "actions": []}', True, "off_schema"),
        ('{"say": "Готово", "actions": [], "mood": "x"}', True, "off_schema"),
    ],
)
async def test_an_answer_that_is_not_one_whole_json_object_is_never_returned(
    contract: Contract, clock: FakeClock, text: str, finished: bool, why: str
) -> None:
    subject, engines, counters = completer(contract)
    engines.completion = replace(engines.completion, text=text, finished=finished)

    with pytest.raises(PermanentFailure) as rejected:
        await subject.complete(request(), deadline(clock))

    assert rejected.value.reason is Reason.MODEL_OUTPUT_INVALID
    assert text not in str(rejected.value)
    assert counters.value("complete_rejected_total", why=why) == 1
    assert counters.value("complete_answers_total") == 0


class HeldEngines(FakeEngines):
    def __init__(self) -> None:
        super().__init__()
        self.entered = asyncio.Semaphore(0)
        self.release = asyncio.Event()

    async def complete(
        self,
        system: str,
        system_marks: Sequence[int],
        messages: Sequence[ChatMessage],
        schema: Mapping[str, object],
        max_tokens: int,
        deadline: Deadline,
    ) -> Completion:
        self.entered.release()
        await self.release.wait()
        return await super().complete(system, system_marks, messages, schema, max_tokens, deadline)


async def test_requests_beyond_the_queue_of_one_model_are_turned_away_at_once(
    contract: Contract, clock: FakeClock
) -> None:
    engines, counters = HeldEngines(), Counters()
    engines.completion = replace(engines.completion, text=ANSWER)
    subject = Completer(engines, contract.complete_limits, LOCAL, MODEL, counters)
    queued = [
        asyncio.ensure_future(subject.complete(request(), deadline(clock)))
        for _ in range(LOCAL.max_in_flight)
    ]
    for _ in queued:
        await engines.entered.acquire()

    with pytest.raises(NotServedHere) as busy:
        await subject.complete(request(), deadline(clock))
    assert busy.value.reason is Reason.DEADLINE_EXCEEDED
    assert counters.value("complete_rejected_total", why="busy") == 1
    assert len(engines.calls) == 0

    engines.release.set()
    answers = await asyncio.gather(*queued)
    assert [answer["text"] for answer in answers] == [ANSWER] * LOCAL.max_in_flight
    assert (await subject.complete(request(), deadline(clock)))["text"] == ANSWER


async def test_a_request_that_cannot_wait_out_the_call_ahead_is_turned_away(
    contract: Contract, clock: FakeClock
) -> None:
    engines, counters = HeldEngines(), Counters()
    engines.completion = replace(engines.completion, text=ANSWER)
    subject = Completer(engines, contract.complete_limits, LOCAL, MODEL, counters)
    slow = asyncio.ensure_future(subject.complete(request(), deadline(clock)))
    await engines.entered.acquire()
    clock.advance(12.0)
    engines.release.set()
    await slow
    engines.release.clear()
    ahead = asyncio.ensure_future(subject.complete(request(), deadline(clock)))
    await engines.entered.acquire()

    with pytest.raises(NotServedHere) as busy:
        await subject.complete(request(), deadline(clock, seconds=20.0))
    roomy = asyncio.ensure_future(subject.complete(request(), deadline(clock, seconds=30.0)))
    await engines.entered.acquire()
    engines.release.set()

    assert busy.value.reason is Reason.DEADLINE_EXCEEDED
    assert counters.value("complete_rejected_total", why="busy") == 1
    assert [answer["text"] for answer in await asyncio.gather(ahead, roomy)] == [ANSWER] * 2


async def test_a_request_the_model_refuses_is_counted(contract: Contract, clock: FakeClock) -> None:
    subject, engines, counters = completer(contract)
    engines.fail("complete", PermanentFailure(Reason.INVALID_REQUEST, "does not fit the context"))

    with pytest.raises(PermanentFailure):
        await subject.complete(request(), deadline(clock))
    with pytest.raises(PermanentFailure):
        await subject.complete(request(max_tokens=0), deadline(clock))

    assert counters.value("complete_rejected_total", why="invalid_request") == 2


async def test_a_worker_with_the_local_model_off_leaves_the_call_to_another_worker(
    contract: Contract, clock: FakeClock
) -> None:
    engines, counters = FakeEngines(), Counters()
    subject = Completer(
        engines, contract.complete_limits, replace(LOCAL, enabled=False), MODEL, counters
    )

    with pytest.raises(NotServedHere) as off:
        await subject.complete(request(), deadline(clock))

    assert off.value.reason is Reason.INTERNAL_ERROR
    assert "off" in str(off.value)
    assert counters.value("complete_unavailable_total", state="off") == 1
    assert engines.calls == []
