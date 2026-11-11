from __future__ import annotations

import math
import time
from collections.abc import Iterator

import numpy as np
import pytest

from tests.fakes.llama import END, FakeGrammar, FakeModel, favouring
from worker.models.local_llm.pace import Pace
from worker.models.local_llm.sampling import pick, write
from worker.runtime.protocol import CALL_GUARD, BadInput, CallExpired

CHUNK = 128


@pytest.fixture(autouse=True)
def unarmed_guard() -> Iterator[None]:
    yield
    CALL_GUARD.expires_at = math.inf


def test_a_short_chunk_does_not_inflate_the_prediction_for_a_full_one() -> None:
    pace = Pace(CHUNK)
    pace.learn(3, 0.3)
    CALL_GUARD.expires_at = time.monotonic() + 2.0
    pace.guard(CHUNK)

    pace.learn(CHUNK, 2.56)
    with pytest.raises(CallExpired):
        pace.guard(CHUNK)
    pace.guard(10)


def test_the_answer_ends_at_the_end_token_and_feeds_each_token_back() -> None:
    model, grammar = FakeModel(), FakeGrammar()
    model.next_logits = favouring(7, 8, END)

    answer, finished = write(model, 5, grammar, 10, Pace(1))

    assert (answer, finished) == ([7, 8], True)
    assert grammar.accepted == [7, 8]
    assert model.evals == [1, 1, 1]


def test_an_answer_cut_at_max_tokens_is_not_finished() -> None:
    model, grammar = FakeModel(), FakeGrammar()
    model.next_logits = favouring(7, 8, 9, END)

    assert write(model, 5, grammar, 2, Pace(1)) == ([7, 8], False)


def test_the_grammar_picks_the_best_allowed_token() -> None:
    model = FakeModel()
    model.next_logits = lambda: np.arange(64, dtype=np.float32)

    assert pick(model, FakeGrammar(lambda token: token < 40)) == 39


def test_a_grammar_that_allows_nothing_fails_before_accepting_a_token() -> None:
    model, grammar = FakeModel(), FakeGrammar(lambda token: False)
    model.next_logits = favouring(7)

    with pytest.raises(BadInput, match="no token"):
        write(model, 5, grammar, 10, Pace(1))
    assert grammar.accepted == []


def test_writing_learns_the_step_time_with_the_pick() -> None:
    model, grammar = FakeModel(), FakeGrammar()
    model.next_logits = favouring(7, 8, END)
    model.on_eval = lambda tokens: time.sleep(0.05)
    pace = Pace(1)
    write(model, 5, grammar, 10, pace)

    CALL_GUARD.expires_at = time.monotonic() + 0.03
    with pytest.raises(CallExpired):
        pace.guard(1)
