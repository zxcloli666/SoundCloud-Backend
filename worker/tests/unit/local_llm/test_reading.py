from __future__ import annotations

import math
import time
from collections.abc import Callable, Iterator, Sequence

import pytest

from tests.fakes.llama import FakeModel
from worker.models.local_llm.pace import Pace
from worker.models.local_llm.prompt import Prompt
from worker.models.local_llm.reading import read
from worker.models.local_llm.states import PromptStates
from worker.runtime.protocol import CALL_GUARD, CallExpired

CHUNK = 4
BYTES_PER_TOKEN = 10


@pytest.fixture(autouse=True)
def unarmed_guard() -> Iterator[None]:
    yield
    CALL_GUARD.expires_at = math.inf


def prompt(system: Sequence[int], *turns: Sequence[int]) -> Prompt:
    tokens = [*system]
    for turn in turns:
        tokens += turn
    return Prompt(tokens, len(system), len(tokens) - 1)


def system(head: int, length: int = 16, tail: int = 0) -> list[int]:
    return [head, *range(100, 100 + length - 2), 200 + tail]


def states(budget_tokens: int = 1000) -> PromptStates[bytes]:
    return PromptStates(budget_tokens * BYTES_PER_TOKEN, CHUNK)


def reader(model: FakeModel, held: PromptStates[bytes]) -> Callable[[Prompt], int]:
    pace = Pace(CHUNK)
    return lambda asked: read(model, held, asked, pace, CHUNK)


def test_alternating_system_texts_stay_reusable_within_the_budget() -> None:
    model, held = FakeModel(bytes_per_token=BYTES_PER_TOKEN), states(budget_tokens=64)
    ask = reader(model, held)
    first, second = system(1), system(2)
    reused = []
    for turn in range(6):
        own = first if turn % 2 == 0 else second
        reused.append(ask(prompt(own, [50 + turn, 60, 61])))
        assert held.held_bytes() <= 64 * BYTES_PER_TOKEN

    assert reused[:2] == [0, 0]
    assert reused[2:] == [16, 16, 16, 16]


def test_a_new_tail_of_a_known_system_keeps_only_the_common_part() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    ask(prompt(system(1, tail=1), [50, 60]))
    ask(prompt(system(1, tail=2), [50, 60]))
    third = ask(prompt(system(1, tail=3), [50, 60]))

    anchors, _ = held.sizes()
    assert anchors == [16, 15]
    assert third == 15


def test_a_state_that_fails_to_load_gives_way_to_a_shallower_one_or_a_cold_read() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    asked = prompt(system(1), [50, 60, 61])
    ask(asked)
    model.failing_restores = 1
    shallower = ask(asked)
    model.failing_restores = 2
    model.evals.clear()

    assert shallower == 16
    assert ask(asked) == 0
    assert sum(model.evals) == len(asked.tokens) - 1


def test_an_expired_read_keeps_what_it_read_for_the_next_try() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    asked = prompt(system(1, length=40), [50, 60, 61])

    def expire_after_two_chunks(tokens: Sequence[int]) -> None:
        if len(model.evals) == 1:
            CALL_GUARD.expires_at = time.monotonic()

    model.on_eval = expire_after_two_chunks
    with pytest.raises(CallExpired):
        ask(asked)
    CALL_GUARD.expires_at = math.inf
    model.on_eval = lambda tokens: None

    assert held.sizes() == ([], [2 * CHUNK])
    assert ask(asked) == 2 * CHUNK
