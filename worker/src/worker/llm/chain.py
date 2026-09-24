from __future__ import annotations

import asyncio
import logging
import math
from collections import deque
from collections.abc import Callable, Coroutine, Mapping
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any

import aiohttp

from worker.domain.deadline import Clock, Deadline
from worker.domain.ports import Grounding
from worker.llm.anthropic import AnthropicProvider
from worker.llm.local import Generator, LocalProvider
from worker.llm.openai_compatible import OpenAiCompatibleProvider
from worker.llm.provider import Provider
from worker.llm.validate import validate
from worker.observability.counters import Counters
from worker.settings import LOCAL_PROVIDER, BreakerSettings, LlmSection, ProviderSettings

RPC_MARGIN_S = 0.5
FALLBACK_RESERVE_S = 1.0
FALLBACK_MIN_BUDGET_S = 2.0

log = logging.getLogger(__name__)

ProviderFactory = Callable[[str, ProviderSettings, aiohttp.ClientSession], Provider]
PROVIDER_KINDS: Mapping[str, ProviderFactory] = {
    "anthropic": AnthropicProvider,
    "openai_compatible": OpenAiCompatibleProvider,
}


class CallOutcome(StrEnum):
    OK = "ok"
    ERROR = "error"
    TIMEOUT = "timeout"
    CLIPPED = "clipped"
    INVALID = "invalid"
    UNGROUNDED = "ungrounded"
    CANCELLED = "cancelled"
    BREAKER_OPEN = "breaker_open"


BREAKER_FAILURES = frozenset({CallOutcome.ERROR, CallOutcome.TIMEOUT, CallOutcome.INVALID})
ANSWERED = frozenset({CallOutcome.OK, CallOutcome.UNGROUNDED})


def build_refiner(
    llm: LlmSection,
    session: aiohttp.ClientSession,
    generator: Generator,
    clock: Clock,
    counters: Counters,
) -> Chain | None:
    primary = link_for(llm.primary, llm, session, generator)
    fallback = link_for(llm.fallback, llm, session, generator)
    log.info(
        "llm providers registered",
        extra={
            "primary": primary.name if primary else None,
            "fallback": fallback.name if fallback else None,
        },
    )
    if primary is None and fallback is None:
        return None
    return Chain(
        primary,
        fallback,
        hedge_after_share=llm.hedge_after_share,
        breaker=llm.breaker,
        clock=clock,
        counters=counters,
    )


def link_for(
    name: str, llm: LlmSection, session: aiohttp.ClientSession, generator: Generator
) -> Link | None:
    if not name:
        return None
    if name == LOCAL_PROVIDER:
        if not llm.local.enabled:
            return None
        return Link(LocalProvider(name, generator), math.inf, math.inf)
    settings = llm.providers[name]
    if not settings.enabled:
        return None
    provider = PROVIDER_KINDS[settings.kind](name, settings, session)
    return Link(provider, settings.soft_timeout_s, settings.hard_timeout_s)


@dataclass(frozen=True)
class Link:
    provider: Provider
    soft_timeout_s: float
    hard_timeout_s: float

    @property
    def name(self) -> str:
        return self.provider.name


class Chain:
    def __init__(
        self,
        primary: Link | None,
        fallback: Link | None,
        *,
        hedge_after_share: float,
        breaker: BreakerSettings,
        clock: Clock,
        counters: Counters,
    ) -> None:
        self.primary = primary
        self.fallback = fallback
        self.hedge_after_share = hedge_after_share
        self.clock = clock
        self.counters = counters
        self.breakers = {
            link.name: Breaker(breaker, clock) for link in (primary, fallback) if link is not None
        }

    async def refine(
        self,
        prompt: str,
        schema: Mapping[str, object],
        grounded: Grounding,
        deadline: Deadline,
    ) -> Mapping[str, object] | None:
        race = Race(self, prompt, schema, grounded, deadline)
        try:
            return await race.run()
        finally:
            await race.stop()

    def admits(self, link: Link) -> bool:
        if not self.breakers[link.name].is_open():
            return True
        self.counters.inc("llm_calls_total", provider=link.name, outcome=CallOutcome.BREAKER_OPEN)
        log.warning("llm provider skipped, breaker open", extra={"provider": link.name})
        return False

    def settle(self, link: Link, result: CallResult, started: float) -> None:
        self.counters.inc("llm_calls_total", provider=link.name, outcome=result.outcome)
        self.counters.observe(
            "llm_latency_ms", (self.clock.now() - started) * 1000.0, provider=link.name
        )
        if result.outcome is not CallOutcome.OK:
            log.warning(
                "llm call failed",
                extra={"provider": link.name, "outcome": result.outcome, "detail": result.detail},
            )
        if self.breakers[link.name].record(result.outcome):
            self.counters.inc("llm_breaker_opened_total", provider=link.name)
            log.error("llm breaker opened", extra={"provider": link.name})


@dataclass(frozen=True)
class CallResult:
    outcome: CallOutcome
    reply: Mapping[str, object] | None = None
    detail: str = ""


@dataclass
class Breaker:
    settings: BreakerSettings
    clock: Clock
    failures: deque[float] = field(default_factory=deque)
    open_until: float = -math.inf
    answered_at: float = -math.inf

    def is_open(self) -> bool:
        return self.clock.now() < self.open_until

    def record(self, outcome: CallOutcome) -> bool:
        now = self.clock.now()
        if outcome in ANSWERED:
            self.answered_at = now
        if not self.counts(outcome, now):
            return False
        self.failures.append(now)
        while self.failures[0] <= now - self.settings.window_s:
            self.failures.popleft()
        if len(self.failures) < self.settings.failures:
            return False
        self.failures.clear()
        self.open_until = now + self.settings.open_s
        return True

    def counts(self, outcome: CallOutcome, now: float) -> bool:
        if outcome is CallOutcome.CLIPPED:
            return self.answered_at <= now - self.settings.window_s
        return outcome in BREAKER_FAILURES


class Race:
    def __init__(
        self,
        chain: Chain,
        prompt: str,
        schema: Mapping[str, object],
        grounded: Grounding,
        deadline: Deadline,
    ) -> None:
        self.chain = chain
        self.clock = chain.clock
        self.prompt = prompt
        self.schema = schema
        self.grounded = grounded
        self.deadline = deadline
        self.chain_end = deadline.minus(RPC_MARGIN_S)
        self.calls: dict[asyncio.Task[CallResult], Link] = {}
        self.end = spawn(self.clock.sleep(self.chain_end.remaining()))
        self.hedge: asyncio.Task[None] | None = None
        self.fallback_pending = chain.fallback is not None

    async def run(self) -> Mapping[str, object] | None:
        primary = self.chain.primary
        if primary is not None and self.chain.admits(primary):
            self.launch(primary, primary.hard_timeout_s, self.chain_end)
            if self.fallback_pending:
                share = self.chain.hedge_after_share * self.deadline.remaining()
                self.hedge = spawn(self.clock.sleep(min(primary.soft_timeout_s, share)))
        else:
            self.start_fallback()
        while self.calls:
            timers = {self.end} if self.hedge is None else {self.end, self.hedge}
            done, _ = await asyncio.wait(
                {*self.calls, *timers}, return_when=asyncio.FIRST_COMPLETED
            )
            if self.end in done:
                return await self.finish_at_end()
            if self.hedge is not None and self.hedge in done:
                self.hedge = None
                self.start_fallback()
            for task in [task for task in self.calls if task in done]:
                self.calls.pop(task)
                result = task.result()
                if result.outcome is CallOutcome.OK:
                    return result.reply
                self.start_fallback()
        return None

    async def finish_at_end(self) -> Mapping[str, object] | None:
        await asyncio.wait(self.calls)
        for task in list(self.calls):
            self.calls.pop(task)
            result = task.result()
            if result.outcome is CallOutcome.OK:
                return result.reply
        self.chain.counters.inc("llm_chain_expired_total")
        return None

    def start_fallback(self) -> None:
        fallback = self.chain.fallback
        if fallback is None or not self.fallback_pending:
            return
        budget = self.deadline.minus(FALLBACK_RESERVE_S)
        if budget.remaining() < FALLBACK_MIN_BUDGET_S:
            return
        self.fallback_pending = False
        if self.chain.admits(fallback):
            self.launch(fallback, fallback.hard_timeout_s, budget)

    def launch(self, link: Link, hard_timeout_s: float, limit: Deadline) -> None:
        own = Deadline(self.clock.now() + hard_timeout_s, self.clock.now)
        clipped = limit.at <= own.at
        call = limit if clipped else own
        self.calls[spawn(self.attempt(link, call, clipped))] = link

    async def attempt(self, link: Link, call: Deadline, clipped: bool) -> CallResult:
        started = self.clock.now()
        result = await self.ask(link, call, clipped)
        self.chain.settle(link, result, started)
        return result

    async def ask(self, link: Link, call: Deadline, clipped: bool) -> CallResult:
        request = spawn(link.provider.complete(self.prompt, self.schema, call))
        timer = spawn(self.clock.sleep(call.remaining()))
        try:
            await asyncio.wait({request, timer}, return_when=asyncio.FIRST_COMPLETED)
        finally:
            await cancel(timer, request)
        if request.cancelled():
            if clipped:
                return CallResult(CallOutcome.CLIPPED, detail="no reply before the rpc deadline")
            return CallResult(CallOutcome.TIMEOUT, detail="no reply within the hard timeout")
        error = request.exception()
        if error is not None:
            detail = f"{type(error).__name__}: {error}"
            if clipped and timed_out(error):
                return CallResult(CallOutcome.CLIPPED, detail=detail)
            return CallResult(CallOutcome.ERROR, detail=detail)
        reply = request.result()
        problems = validate(reply, self.schema)
        if problems:
            return CallResult(CallOutcome.INVALID, detail="; ".join(problems[:3]))
        if not self.grounded(reply):
            return CallResult(CallOutcome.UNGROUNDED)
        return CallResult(CallOutcome.OK, reply)

    async def stop(self) -> None:
        for task, link in self.calls.items():
            if not task.done():
                self.chain.counters.inc(
                    "llm_calls_total", provider=link.name, outcome=CallOutcome.CANCELLED
                )
        timers = [self.end] if self.hedge is None else [self.end, self.hedge]
        await cancel(*self.calls, *timers)


def timed_out(error: BaseException) -> bool:
    return isinstance(error, TimeoutError) or isinstance(error.__cause__, TimeoutError)


def spawn[T](work: Coroutine[Any, Any, T]) -> asyncio.Task[T]:
    return asyncio.create_task(work)


async def cancel(*tasks: asyncio.Future[Any]) -> None:
    for task in tasks:
        task.cancel()
    await asyncio.gather(*tasks, return_exceptions=True)
