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
    stable = states.stable_prefix(tokens, prompt.shared)
    reused = restore(model, states, tokens)
    system = stable or prompt.shared
    try:
        for mark, anchor in ((system, True), (prompt.turn, False)):
            if model.read_tokens < mark:
                evaluate(model, tokens, mark, pace, chunk_tokens)
                states.keep(tokens[:mark], model.save(), anchor)
        evaluate(model, tokens, len(tokens) - 1, pace, chunk_tokens)
    except CallExpired:
        if model.read_tokens > reused:
            states.keep(tokens[: model.read_tokens], model.save(), False)
        raise
    return reused


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
    for start in range(model.read_tokens, stop, chunk_tokens):
        chunk = tokens[start : min(start + chunk_tokens, stop)]
        pace.guard(len(chunk))
        started = time.monotonic()
        model.eval(chunk)
        pace.learn(len(chunk), time.monotonic() - started)
