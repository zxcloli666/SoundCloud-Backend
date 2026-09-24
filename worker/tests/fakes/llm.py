from __future__ import annotations

from collections import deque
from collections.abc import Mapping
from dataclasses import dataclass, field

from tests.fakes.clock import FakeClock
from worker.domain.deadline import Deadline
from worker.domain.ports import Grounding


class FakeProviderError(Exception):
    pass


@dataclass
class Scripted:
    reply: Mapping[str, object] | None = None
    error: Exception | None = None
    delay_s: float = 0.0


@dataclass
class FakeProvider:
    name: str
    clock: FakeClock
    script: deque[Scripted] = field(default_factory=deque)
    calls: list[tuple[str, Mapping[str, object]]] = field(default_factory=list)
    cancelled: int = 0

    def reply_with(self, reply: Mapping[str, object], delay_s: float = 0.0) -> None:
        self.script.append(Scripted(reply=reply, delay_s=delay_s))

    def fail_with(self, error: Exception, delay_s: float = 0.0) -> None:
        self.script.append(Scripted(error=error, delay_s=delay_s))

    def hang(self) -> None:
        self.script.append(Scripted(delay_s=float("inf")))

    async def complete(
        self, prompt: str, schema: Mapping[str, object], deadline: Deadline
    ) -> Mapping[str, object]:
        self.calls.append((prompt, schema))
        if not self.script:
            raise FakeProviderError(f"{self.name}: no scripted reply")
        step = self.script.popleft()
        try:
            if step.delay_s == float("inf"):
                await self.clock.sleep(deadline.remaining() + 3600)
            elif step.delay_s > 0:
                await self.clock.sleep(step.delay_s)
        except BaseException:
            self.cancelled += 1
            raise
        if step.error is not None:
            raise step.error
        if step.reply is None:
            raise FakeProviderError(f"{self.name}: hung past the deadline")
        return step.reply


@dataclass
class FakeRefiner:
    replies: deque[Mapping[str, object] | None] = field(default_factory=deque)
    calls: list[tuple[str, Mapping[str, object]]] = field(default_factory=list)
    ungrounded: int = 0

    def reply_with(self, reply: Mapping[str, object] | None) -> None:
        self.replies.append(reply)

    async def refine(
        self,
        prompt: str,
        schema: Mapping[str, object],
        grounded: Grounding,
        deadline: Deadline,
    ) -> Mapping[str, object] | None:
        deadline.check("refine")
        self.calls.append((prompt, schema))
        reply = self.replies.popleft() if self.replies else None
        if reply is not None and not grounded(reply):
            self.ungrounded += 1
            return None
        return reply
