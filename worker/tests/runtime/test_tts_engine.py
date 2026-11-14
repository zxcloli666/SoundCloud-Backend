from __future__ import annotations

import time
from collections.abc import AsyncIterator
from dataclasses import dataclass
from pathlib import Path

import pytest

from tests.runtime.support import TEST_POLICY, launch, quiet_log, settled, wait_until
from tests.unit.tts import fake_qwen
from tests.unit.tts.engine_home import OPTIONS, SPOKEN, install, journal, voice
from worker.app import SLOT_LOADERS
from worker.domain.deadline import Deadline
from worker.domain.outcome import Reason, TransientFailure
from worker.engines import EngineSlots, RuntimeEngines
from worker.fetch_models import TTS_HOME_ENV
from worker.observability.counters import Counters
from worker.runtime.protocol import SlotSpec
from worker.runtime.supervisor import EnginePlan, Supervisor, budget_plans
from worker.settings import TTS_DEVICE, TTS_SLOT

TEXT = "Громкость на семьдесят."
IDLE_S = 1.0
ENGINE_THREADS = 6
CPUS_IN_THE_AFFINITY_MASK = 2


@dataclass
class Running:
    home: Path
    plan: EnginePlan
    supervisor: Supervisor
    engines: RuntimeEngines
    counters: Counters


@pytest.fixture
async def running(tmp_path: Path) -> AsyncIterator[Running]:
    home = install(tmp_path)
    spec = SlotSpec(TTS_SLOT, SLOT_LOADERS[TTS_SLOT], "", "", TTS_DEVICE, 1, 0, OPTIONS)
    own = EnginePlan(
        TTS_SLOT,
        (spec,),
        reserved=True,
        threads=ENGINE_THREADS,
        own_threads=True,
        idle_unload_s=IDLE_S,
    )
    [plan] = budget_plans([own], CPUS_IN_THE_AFFINITY_MASK)
    counters = Counters()
    supervisor = Supervisor(
        [plan],
        TEST_POLICY,
        counters,
        launch=launch(**{TTS_HOME_ENV: str(home)}),
        log=quiet_log(),
    )
    await supervisor.start()
    await settled(supervisor, supervisor.slots)
    engines = RuntimeEngines(EngineSlots(supervisor, {}, {}, lambda: None, {}), priority=True)
    yield Running(home, plan, supervisor, engines, counters)
    await supervisor.stop()


async def test_a_line_and_a_voice_switch_go_through_the_engine_process(running: Running) -> None:
    first = await running.engines.say("loli", TEXT, Deadline.after(30))
    again = await running.engines.say("loli", TEXT, Deadline.after(30))
    other = await running.engines.say("moriarty", TEXT, Deadline.after(30))

    assert [speech.samples.size for speech in (first, again, other)] == [SPOKEN] * 3
    assert [speech.loaded for speech in (first, again, other)] == [True, False, True]
    assert len(journal(running.home, "start")) == 2
    assert len(journal(running.home, "exit")) == 1


async def test_the_deadline_cancels_the_line_and_keeps_the_engine(running: Running) -> None:
    voice(running.home, "loli", "slow")
    await running.engines.say("loli", TEXT, Deadline.after(30))
    before = running.supervisor.engines()

    with pytest.raises(TransientFailure) as failure:
        await running.engines.say("loli", TEXT, Deadline.after(1.5))

    assert failure.value.reason is Reason.DEADLINE_EXCEEDED
    assert [cancel["id"] for cancel in journal(running.home, "cancel")] == [2]
    assert running.supervisor.engines() == before
    assert running.counters.value("slot_kills_deadline_total", slot=TTS_SLOT) == 0
    assert len(journal(running.home, "start")) == 1


async def test_an_idle_voice_is_unloaded_and_loaded_again_on_the_next_line(
    running: Running,
) -> None:
    await running.engines.say("loli", TEXT, Deadline.after(30))

    assert await wait_until(lambda: len(journal(running.home, "exit")) == 1, IDLE_S * 10)
    speech = await running.engines.say("loli", TEXT, Deadline.after(30))

    assert speech.loaded is True
    assert len(journal(running.home, "start")) == 2


async def test_a_voice_that_cannot_start_fails_the_line_and_the_next_one_retries(
    running: Running,
) -> None:
    voice(running.home, "loli", "fail_start")

    with pytest.raises(TransientFailure) as failure:
        await running.engines.say("loli", TEXT, Deadline.after(30))
    voice(running.home, "loli", "speak")
    speech = await running.engines.say("loli", TEXT, Deadline.after(30))

    assert failure.value.reason is Reason.INTERNAL_ERROR
    assert "not_ready" in str(failure.value)
    assert speech.samples.size == SPOKEN


async def test_the_supervisor_shows_and_leases_the_threads_the_engine_runs(
    running: Running,
) -> None:
    assert running.plan.threads == ENGINE_THREADS
    assert running.supervisor.threads_label(running.plan) == str(ENGINE_THREADS)
    for priority in (True, False):
        client = await running.supervisor.acquire(
            TTS_SLOT, time.monotonic() + 10, priority=priority
        )
        try:
            assert client.call_threads == ENGINE_THREADS
        finally:
            running.supervisor.release(client)


async def test_the_engine_memory_counts_the_running_voice(running: Running) -> None:
    alone = running.supervisor.memory()[TTS_SLOT]
    voice(running.home, "loli", "heavy")

    await running.engines.say("loli", TEXT, Deadline.after(30))

    grown = running.supervisor.memory()[TTS_SLOT]
    report = running.supervisor.snapshot()[TTS_SLOT]
    assert grown.resident_mib - alone.resident_mib >= fake_qwen.BALLAST_MIB
    assert grown.charged_mib - alone.charged_mib >= fake_qwen.BALLAST_MIB
    assert report["charged_mib"] - alone.charged_mib >= fake_qwen.BALLAST_MIB
