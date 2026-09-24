from __future__ import annotations

import asyncio
from collections.abc import Mapping

import aiohttp
import pytest

from tests.conftest import CONFIG_DIR
from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.llm import FakeProvider, FakeProviderError
from worker import settings as settings_module
from worker.domain.deadline import Deadline
from worker.llm.chain import Chain, Link, build_refiner
from worker.observability.counters import Counters
from worker.settings import BreakerSettings

SCHEMA: Mapping[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["name"],
    "properties": {"name": {"type": "string"}},
}
INPUT_NAMES = {"Kendrick Lamar", "SZA"}
BREAKER = BreakerSettings(failures=5, window_s=60, open_s=120)
STEP_S = 0.125
TOLERANCE_S = 1.5 * STEP_S


def grounded(reply: Mapping[str, object]) -> bool:
    return reply.get("name") in INPUT_NAMES


class Rig:
    def __init__(self, clock: FakeClock, *, fallback: bool = True) -> None:
        self.clock = clock
        self.counters = Counters()
        self.primary = FakeProvider("anthropic", clock)
        self.fallback = FakeProvider("openai_compatible", clock)
        self.chain = Chain(
            Link(self.primary, 4.0, 6.0),
            Link(self.fallback, 4.0, 8.0) if fallback else None,
            hedge_after_share=0.35,
            breaker=BREAKER,
            clock=clock,
            counters=self.counters,
        )

    async def refine(self, remaining_s: float) -> tuple[Mapping[str, object] | None, float]:
        started = self.clock.now()
        deadline = Deadline(started + remaining_s, self.clock.now)
        task = asyncio.create_task(self.chain.refine("who?", SCHEMA, grounded, deadline))
        await self.clock.tick()
        while not task.done():
            await self.clock.tick(STEP_S)
        return task.result(), self.clock.now() - started

    def calls(self, provider: str, outcome: str) -> int:
        return self.counters.value("llm_calls_total", provider=provider, outcome=outcome)


async def test_a_fast_primary_answer_wins_alone(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.reply_with({"name": "SZA"}, delay_s=1.0)

    reply, elapsed = await rig.refine(18.0)

    assert reply == {"name": "SZA"}
    assert elapsed == pytest.approx(1.0, abs=TOLERANCE_S)
    assert rig.fallback.calls == []
    assert rig.calls("anthropic", "ok") == 1


async def test_slow_primary_is_hedged_at_the_soft_timeout(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.reply_with({"name": "SZA"}, delay_s=5.5)
    rig.fallback.reply_with({"name": "Kendrick Lamar"}, delay_s=1.0)

    reply, elapsed = await rig.refine(18.0)

    assert reply == {"name": "Kendrick Lamar"}
    assert elapsed == pytest.approx(5.0, abs=TOLERANCE_S)
    assert rig.primary.cancelled == 1
    assert rig.calls("anthropic", "cancelled") == 1
    assert rig.calls("openai_compatible", "ok") == 1


async def test_hedge_point_shrinks_with_the_remaining_time(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.hang()
    rig.fallback.reply_with({"name": "SZA"})

    reply, elapsed = await rig.refine(6.0)

    assert reply == {"name": "SZA"}
    assert elapsed == pytest.approx(0.35 * 6.0, abs=TOLERANCE_S)


async def test_ungrounded_primary_starts_the_fallback_at_once(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.reply_with({"name": "Drake"}, delay_s=0.5)
    rig.fallback.reply_with({"name": "SZA"}, delay_s=0.5)

    reply, elapsed = await rig.refine(18.0)

    assert reply == {"name": "SZA"}
    assert elapsed == pytest.approx(1.0, abs=TOLERANCE_S)
    assert rig.calls("anthropic", "ungrounded") == 1


@pytest.mark.parametrize(
    ("primary_reply", "outcome"),
    [({"name": 7}, "invalid"), ({"name": "SZA", "extra": True}, "invalid"), (None, "error")],
)
async def test_invalid_or_failed_primary_starts_the_fallback(
    clock: FakeClock, primary_reply: Mapping[str, object] | None, outcome: str
) -> None:
    rig = Rig(clock)
    if primary_reply is None:
        rig.primary.fail_with(FakeProviderError("http 500"), delay_s=0.2)
    else:
        rig.primary.reply_with(primary_reply, delay_s=0.2)
    rig.fallback.reply_with({"name": "SZA"})

    reply, _ = await rig.refine(18.0)

    assert reply == {"name": "SZA"}
    assert rig.calls("anthropic", outcome) == 1


async def test_hanging_primary_with_little_time_gives_up_before_the_caller(
    clock: FakeClock,
) -> None:
    rig = Rig(clock)
    rig.primary.hang()

    reply, elapsed = await rig.refine(4.0)

    assert reply is None
    assert elapsed <= 3.5 + TOLERANCE_S
    assert rig.fallback.calls == []
    assert rig.counters.value("llm_chain_expired_total") == 1


async def test_an_answer_landing_on_the_chain_end_is_kept(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.reply_with({"name": "SZA"}, delay_s=3.5)

    reply, _ = await rig.refine(4.0)

    assert reply == {"name": "SZA"}
    assert rig.calls("anthropic", "ok") == 1
    assert rig.counters.value("llm_chain_expired_total") == 0


async def test_primary_hanging_in_short_windows_opens_the_breaker(clock: FakeClock) -> None:
    rig = Rig(clock, fallback=False)
    for _ in range(BREAKER.failures):
        rig.primary.hang()
        reply, _ = await rig.refine(4.0)
        assert reply is None

    assert rig.calls("anthropic", "clipped") == BREAKER.failures
    assert rig.calls("anthropic", "cancelled") == 0
    assert rig.counters.value("llm_breaker_opened_total", provider="anthropic") == 1


def transport_timeout() -> FakeProviderError:
    error = FakeProviderError("transport: TimeoutError")
    error.__cause__ = TimeoutError()
    return error


@pytest.mark.parametrize("clip", ["timer", "transport"])
async def test_calls_clipped_by_the_rpc_deadline_spare_an_answering_provider(
    clock: FakeClock, clip: str
) -> None:
    rig = Rig(clock, fallback=False)
    rig.primary.reply_with({"name": "SZA"}, delay_s=3.5)
    assert (await rig.refine(18.0))[0] == {"name": "SZA"}

    for _ in range(BREAKER.failures):
        if clip == "timer":
            rig.primary.reply_with({"name": "SZA"}, delay_s=3.5)
        else:
            rig.primary.fail_with(transport_timeout(), delay_s=3.0)
        reply, _ = await rig.refine(3.6)
        assert reply is None
    rig.primary.reply_with({"name": "Kendrick Lamar"}, delay_s=1.0)

    reply, _ = await rig.refine(18.0)

    assert reply == {"name": "Kendrick Lamar"}
    assert rig.calls("anthropic", "clipped") == BREAKER.failures
    assert rig.calls("anthropic", "timeout") == rig.calls("anthropic", "error") == 0
    assert rig.counters.value("llm_breaker_opened_total", provider="anthropic") == 0


async def test_a_clipped_call_failing_for_real_still_counts_as_an_error(clock: FakeClock) -> None:
    rig = Rig(clock, fallback=False)
    rig.primary.fail_with(FakeProviderError("http 500"), delay_s=0.5)

    await rig.refine(3.6)

    assert rig.calls("anthropic", "error") == 1
    assert rig.calls("anthropic", "clipped") == 0


async def test_fallback_is_not_started_without_two_seconds_left(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.fail_with(FakeProviderError("boom"), delay_s=0.25)

    reply, elapsed = await rig.refine(3.2)

    assert reply is None
    assert rig.fallback.calls == []
    assert elapsed == pytest.approx(0.25, abs=TOLERANCE_S)


async def test_both_exhausted_returns_none(clock: FakeClock) -> None:
    rig = Rig(clock)
    rig.primary.reply_with({"name": "Drake"})
    rig.fallback.reply_with({"name": "Drake"})

    reply, _ = await rig.refine(18.0)

    assert reply is None
    assert rig.calls("openai_compatible", "ungrounded") == 1


async def test_primary_hard_timeout_counts_as_timeout(clock: FakeClock) -> None:
    rig = Rig(clock, fallback=False)
    rig.primary.hang()

    reply, elapsed = await rig.refine(18.0)

    assert reply is None
    assert elapsed == pytest.approx(6.0, abs=TOLERANCE_S)
    assert rig.calls("anthropic", "timeout") == 1


async def test_breaker_opens_after_five_failures_and_closes_later(clock: FakeClock) -> None:
    rig = Rig(clock)
    for _ in range(5):
        rig.primary.fail_with(FakeProviderError("http 529"))
        rig.fallback.reply_with({"name": "SZA"})
        await rig.refine(18.0)
    rig.fallback.reply_with({"name": "SZA"})

    reply, elapsed = await rig.refine(18.0)

    assert reply == {"name": "SZA"}
    assert elapsed < 0.5
    assert len(rig.primary.calls) == 5
    assert rig.counters.value("llm_breaker_opened_total", provider="anthropic") == 1
    assert rig.calls("anthropic", "breaker_open") == 1

    clock.advance(121.0)
    rig.primary.reply_with({"name": "Kendrick Lamar"})
    reply, _ = await rig.refine(18.0)
    assert reply == {"name": "Kendrick Lamar"}


async def test_failures_spread_beyond_the_window_do_not_open_the_breaker(
    clock: FakeClock,
) -> None:
    rig = Rig(clock, fallback=False)
    for _ in range(6):
        rig.primary.fail_with(FakeProviderError("http 500"))
        await rig.refine(18.0)
        clock.advance(15.0)

    assert rig.counters.value("llm_breaker_opened_total", provider="anthropic") == 0


async def test_only_the_fallback_answers_when_the_primary_is_absent(clock: FakeClock) -> None:
    counters = Counters()
    fallback = FakeProvider("openai_compatible", clock)
    fallback.reply_with({"name": "SZA"})
    chain = Chain(
        None,
        Link(fallback, 4.0, 8.0),
        hedge_after_share=0.35,
        breaker=BREAKER,
        clock=clock,
        counters=counters,
    )
    deadline = Deadline(clock.now() + 10.0, clock.now)

    assert await chain.refine("who?", SCHEMA, grounded, deadline) == {"name": "SZA"}


async def test_without_keys_no_refiner_is_built(base_env: dict[str, str], clock: FakeClock) -> None:
    settings = settings_module.load(CONFIG_DIR, base_env)

    async with aiohttp.ClientSession() as session:
        refiner = build_refiner(settings.llm, session, FakeEngines(), clock, Counters())

    assert refiner is None


async def test_only_providers_with_keys_are_registered(
    base_env: dict[str, str], clock: FakeClock
) -> None:
    settings = settings_module.load(CONFIG_DIR, {**base_env, "ANTHROPIC_API_KEY": "sk-test"})

    async with aiohttp.ClientSession() as session:
        refiner = build_refiner(settings.llm, session, FakeEngines(), clock, Counters())

    assert refiner is not None
    assert refiner.primary is not None
    assert refiner.primary.name == "anthropic"
    assert (refiner.primary.soft_timeout_s, refiner.primary.hard_timeout_s) == (4, 6)
    assert refiner.fallback is None


async def test_local_provider_is_registered_only_when_enabled(
    base_env: dict[str, str], clock: FakeClock
) -> None:
    env = {
        **base_env,
        "ANTHROPIC_API_KEY": "sk-test",
        "WORKER__LLM__FALLBACK": "local",
        "WORKER__LLM__LOCAL__ENABLED": "true",
    }
    settings = settings_module.load(CONFIG_DIR, env)
    engines = FakeEngines()
    engines.generated = '{"name": "SZA"}'

    async with aiohttp.ClientSession() as session:
        refiner = build_refiner(settings.llm, session, engines, clock, Counters())
        assert refiner is not None
        assert refiner.fallback is not None
        assert refiner.fallback.name == "local"
        reply = await refiner.fallback.provider.complete(
            "who?", SCHEMA, Deadline(clock.now() + 10.0, clock.now)
        )

    assert reply == {"name": "SZA"}
    assert engines.calls[0][0] == "generate"
