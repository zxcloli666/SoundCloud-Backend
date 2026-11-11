from __future__ import annotations

from collections import OrderedDict
from collections.abc import Sequence, Sized

Prefix = tuple[int, ...]


class PromptStates[S: Sized]:
    def __init__(self, budget_bytes: int, min_tokens: int) -> None:
        self._budget_bytes = budget_bytes
        self._min_tokens = min_tokens
        self._anchors: OrderedDict[Prefix, S] = OrderedDict()
        self._turns: OrderedDict[Prefix, S] = OrderedDict()
        self._unfinished: set[Prefix] = set()

    def sizes(self) -> tuple[list[int], list[int]]:
        return [len(prefix) for prefix in self._anchors], [len(prefix) for prefix in self._turns]

    def held_bytes(self) -> int:
        return sum(len(state) for held in (self._anchors, self._turns) for state in held.values())

    def deepest(self, tokens: Sequence[int]) -> tuple[Prefix, S] | None:
        found: tuple[Prefix, S] | None = None
        for held in (self._anchors, self._turns):
            for prefix in [prefix for prefix in held if starts_with(tokens, prefix)]:
                held.move_to_end(prefix)
                if found is None or len(prefix) > len(found[0]):
                    found = (prefix, held[prefix])
        return found

    def stable_prefix(self, tokens: Sequence[int], shared: int) -> int:
        common = max(
            (
                common_length(tokens, prefix)
                for held in (self._anchors, self._turns)
                for prefix in held
                if not starts_with(tokens, prefix)
            ),
            default=0,
        )
        return common if self._min_tokens <= common < shared else 0

    def keep(self, tokens: Sequence[int], state: S, anchor: bool, unfinished: bool = False) -> None:
        if len(tokens) < self._min_tokens:
            return
        prefix = tuple(tokens)
        if prefix in self._anchors:
            if anchor and not unfinished:
                self._unfinished.discard(prefix)
            return
        for outgrown in [
            held for held in (*self._turns, *self._unfinished) if starts_with(prefix, held)
        ]:
            self.drop(outgrown)
        self._turns.pop(prefix, None)
        held = self._anchors if anchor else self._turns
        held[prefix] = state
        if anchor and unfinished:
            self._unfinished.add(prefix)
        self._fit()

    def drop(self, prefix: Prefix) -> None:
        self._anchors.pop(prefix, None)
        self._turns.pop(prefix, None)
        self._unfinished.discard(prefix)

    def forget_turns(self) -> None:
        self._turns.clear()

    def _fit(self) -> None:
        while self.held_bytes() > self._budget_bytes:
            self.drop(next(iter(self._turns or self._last_anchors())))

    def _last_anchors(self) -> list[Prefix]:
        return [
            prefix
            for prefix in self._anchors
            if not any(starts_with(longer, prefix) for longer in self._anchors)
        ]


def starts_with(tokens: Sequence[int], prefix: Prefix) -> bool:
    return len(prefix) < len(tokens) and tuple(tokens[: len(prefix)]) == prefix


def common_length(tokens: Sequence[int], prefix: Prefix) -> int:
    length = 0
    for ours, theirs in zip(tokens, prefix, strict=False):
        if ours != theirs:
            break
        length += 1
    return length
