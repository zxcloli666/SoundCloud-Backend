from __future__ import annotations

import math
import time
from collections.abc import Callable, Iterator, Sequence
from dataclasses import replace

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


def marked(marks: Sequence[int], system: Sequence[int], *turns: Sequence[int]) -> Prompt:
    return replace(prompt(system, *turns), marks=tuple(marks))


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


def test_a_read_keeps_a_state_at_every_mark_and_a_new_tail_goes_on_from_the_last_one() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    first = ask(marked([8, 12], system(1, tail=1), [50, 60]))
    kept = held.sizes()
    second = ask(marked([8, 12], system(1, tail=2), [50, 60]))

    assert (first, second) == (0, 12)
    assert kept == ([8, 12], [17])
    assert held.sizes()[0] == [8, 12]


def test_the_end_of_a_marked_system_is_kept_only_when_it_is_marked() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    ask(marked([8], system(1), [50, 60]))
    without = ask(marked([8], system(1), [51, 60]))
    ask(marked([8, 16], system(2), [50, 60]))
    marked_end = ask(marked([8, 16], system(2), [51, 60]))

    assert (without, marked_end) == (8, 16)


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


def expire_after(model: FakeModel, chunks: int) -> None:
    def expire(tokens: Sequence[int]) -> None:
        if len(model.evals) == chunks - 1:
            CALL_GUARD.expires_at = time.monotonic()

    model.evals.clear()
    model.on_eval = expire


def expired(model: FakeModel, ask: Callable[[Prompt], int], asked: Prompt, chunks: int) -> None:
    expire_after(model, chunks)
    with pytest.raises(CallExpired):
        ask(asked)
    CALL_GUARD.expires_at = math.inf
    model.on_eval = lambda tokens: None


def test_an_expired_read_keeps_what_it_read_and_the_next_try_anchors_the_whole_system() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    own = system(1, length=40)
    asked = prompt(own, [50, 60, 61])
    expired(model, ask, asked, chunks=2)

    assert held.sizes() == ([2 * CHUNK], [])
    assert ask(asked) == 2 * CHUNK
    assert held.sizes()[0] == [len(own)]
    assert ask(prompt(own, [51, 60, 61])) == len(own)


def test_a_system_read_cut_into_calls_goes_on_after_each_unload() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    own = system(1, length=40)
    asked = prompt(own, [50, 60, 61])
    resumed_at = []
    for _ in range(3):
        expired(model, ask, asked, chunks=2)
        held.forget_turns()
        resumed_at.append(held.sizes())
    answered_from = ask(asked)
    held.forget_turns()

    assert resumed_at == [([2 * CHUNK], []), ([4 * CHUNK], []), ([6 * CHUNK], [])]
    assert answered_from == 6 * CHUNK
    assert held.sizes() == ([len(own)], [])


def test_a_read_cut_short_after_the_system_is_kept_only_until_the_unload() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    own = system(1, length=2 * CHUNK)
    asked = prompt(own, [50, *range(60, 60 + 3 * CHUNK)])
    expired(model, ask, asked, chunks=3)
    kept = held.sizes()
    held.forget_turns()

    assert kept == ([len(own)], [3 * CHUNK])
    assert held.sizes() == ([len(own)], [])


def test_one_slow_chunk_does_not_refuse_every_read_after_it() -> None:
    chunk, sample = 32, 16
    model, held = FakeModel(), states()
    pace = Pace(sample)
    pace.learn(chunk, chunk * 1.0)
    asked = prompt(system(1, length=100), [50, 60, 61])
    CALL_GUARD.expires_at = time.monotonic() + 24.0

    read(model, held, asked, pace, chunk)

    assert sample <= model.evals[0] < chunk
    assert model.evals[1] == chunk
    assert sum(model.evals) == len(asked.tokens) - 1


def test_a_read_cut_short_after_a_mark_keeps_the_mark_and_what_it_read() -> None:
    model, held = FakeModel(), states()
    ask = reader(model, held)
    own = system(1, length=40)
    asked = marked([2 * CHUNK], own, [50, 60, 61])
    expired(model, ask, asked, chunks=3)
    kept = held.sizes()
    resumed_at = ask(asked)
    held.forget_turns()

    assert kept == ([2 * CHUNK, 3 * CHUNK], [])
    assert resumed_at == 3 * CHUNK
    assert held.sizes() == ([2 * CHUNK], [])
