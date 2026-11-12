from __future__ import annotations

import time
from collections.abc import Sequence

from worker.models.local_llm.native import Model, State
from worker.models.local_llm.pace import Pace
from worker.models.local_llm.prompt import Prompt
from worker.models.local_llm.states import PromptStates
from worker.observability.logging import JsonLog
from worker.runtime.protocol import CallExpired
from worker.settings import LOCAL_LLM_SLOT

log = JsonLog(component=LOCAL_LLM_SLOT)


def read(
    model: Model, states: PromptStates[State], prompt: Prompt, pace: Pace, chunk_tokens: int
) -> int:
    tokens = prompt.tokens
    marks = prompt.marks or unmarked_stops(states, tokens, prompt.shared)
    reused = restore(model, states, tokens)
    try:
        for stop, anchor in (*((mark, True) for mark in marks), (prompt.turn, False)):
            if model.read_tokens < stop:
                evaluate(model, tokens, stop, pace, chunk_tokens)
                states.keep(tokens[:stop], model.save(), anchor)
        evaluate(model, tokens, len(tokens) - 1, pace, chunk_tokens)
    except CallExpired:
        if model.read_tokens > reused:
            inside_system = model.read_tokens < prompt.shared
            states.keep(
                tokens[: model.read_tokens], model.save(), inside_system, unfinished=inside_system
            )
        raise
    return reused


def unmarked_stops(
    states: PromptStates[State], tokens: Sequence[int], shared: int
) -> tuple[int, ...]:
    stable = states.stable_prefix(tokens, shared)
    return (stable, shared) if stable else (shared,)


def restore(model: Model, states: PromptStates[State], tokens: Sequence[int]) -> int:
    model.clear()
    while (held := states.deepest(tokens)) is not None:
        prefix, state = held
        if model.restore(state, len(prefix)):
            return len(prefix)
        states.drop(prefix)
        log.warning("llm_state_restore_failed", tokens=len(prefix), bytes=len(state))
    return 0


def evaluate(model: Model, tokens: Sequence[int], stop: int, pace: Pace, chunk_tokens: int) -> None:
    while model.read_tokens < stop:
        start = model.read_tokens
        chunk = tokens[start : start + pace.fitting(min(chunk_tokens, stop - start))]
        started = time.monotonic()
        model.eval(chunk)
        pace.learn(len(chunk), time.monotonic() - started)
