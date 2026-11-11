from __future__ import annotations

import time

import numpy as np

from worker.models.local_llm.native import TOKEN_DATA, Grammar, Model
from worker.models.local_llm.pace import Pace
from worker.runtime.protocol import BadInput


def write(
    model: Model, last: int, grammar: Grammar, max_tokens: int, pace: Pace
) -> tuple[list[int], bool]:
    answer: list[int] = []
    pending = last
    while True:
        pace.guard(1)
        started = time.monotonic()
        model.eval([pending])
        token = pick(model, grammar)
        pace.learn(1, time.monotonic() - started)
        if model.is_end(token):
            return answer, True
        if len(answer) == max_tokens:
            return answer, False
        grammar.accept(token)
        answer.append(token)
        pending = token


def pick(model: Model, grammar: Grammar) -> int:
    logits = model.logits()
    favourite = np.zeros(1, dtype=TOKEN_DATA)
    favourite["id"] = logits.argmax()
    grammar.constrain(favourite)
    if np.isfinite(favourite["logit"][0]):
        return int(favourite["id"][0])
    candidates = np.zeros(len(logits), dtype=TOKEN_DATA)
    candidates["id"] = np.arange(len(logits))
    candidates["logit"] = logits
    grammar.constrain(candidates)
    best = int(candidates["logit"].argmax())
    if not np.isfinite(candidates["logit"][best]):
        raise BadInput("the schema leaves no token to continue the answer")
    return best
