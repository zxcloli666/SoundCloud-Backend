from __future__ import annotations

from collections.abc import Callable, Sequence

import numpy as np

END = 0
VOCAB = 64


class FakeModel:
    def __init__(self, *, context_tokens: int = 4096, bytes_per_token: int = 10) -> None:
        self.context_tokens = context_tokens
        self.bytes_per_token = bytes_per_token
        self.vocab_size = VOCAB
        self.markers: frozenset[int] = frozenset()
        self.read_tokens = 0
        self.evals: list[int] = []
        self.failing_restores = 0
        self.next_logits: Callable[[], np.ndarray] = lambda: np.zeros(VOCAB, np.float32)
        self.on_eval: Callable[[Sequence[int]], None] = lambda tokens: None

    def eval(self, tokens: Sequence[int]) -> None:
        self.on_eval(tokens)
        self.evals.append(len(tokens))
        self.read_tokens += len(tokens)

    def clear(self) -> None:
        self.read_tokens = 0

    def save(self) -> bytes:
        return bytes(self.read_tokens * self.bytes_per_token)

    def restore(self, state: bytes, tokens: int) -> bool:
        if self.failing_restores:
            self.failing_restores -= 1
            self.clear()
            return False
        self.read_tokens = tokens
        return True

    def logits(self) -> np.ndarray:
        return self.next_logits()

    def is_end(self, token: int) -> bool:
        return token == END


class FakeGrammar:
    def __init__(self, allowed: Callable[[int], bool] = lambda token: True) -> None:
        self.allowed = allowed
        self.accepted: list[int] = []

    def constrain(self, candidates: np.ndarray) -> None:
        for row in candidates:
            if not self.allowed(int(row["id"])):
                row["logit"] = -np.inf

    def accept(self, token: int) -> None:
        self.accepted.append(token)


def favouring(*tokens: int) -> Callable[[], np.ndarray]:
    steps = iter(tokens)

    def logits() -> np.ndarray:
        row = np.zeros(VOCAB, np.float32)
        row[next(steps)] = 10.0
        return row

    return logits
