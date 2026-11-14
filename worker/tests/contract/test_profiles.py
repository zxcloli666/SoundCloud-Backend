from __future__ import annotations

import os
from dataclasses import dataclass

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR, LLM_KEYS
from worker import app
from worker import settings as s
from worker.app import TTS_THREADS_ENV, Blueprint, StartupError, tts_options
from worker.contract import Contract
from worker.fetch_models import FASTTEXT_HOME_ENV, TTS_HOME_ENV
from worker.runtime import devices
from worker.runtime.supervisor import BULK_NICE

TRUSTED_LANES = {
    "audio": 16,
    "lyrics": 32,
    "transcribe": 4,
    "encode": 32,
    "collab": 1,
    "taste": 1,
    "ai": 64,
}
SYNC_X2 = {"sep": 2, "asr": 2, "align": 2, "mms": 1}
SLOT_MODE_MAX_BATCH = {
    "muq": 4,
    "mulan": 8,
    "text": 16000,
    "sep": 2,
    "asr": 4,
    "align": 4,
    "mms": 8,
}


@dataclass(frozen=True)
class Profile:
    mode: str
    trust: str
    lanes: dict[str, int]
    replicas: dict[str, int]
    local_llm: bool = False


DESIGN = {
    "gpu-12": Profile(
        "lane", "public", {"audio": 4, "lyrics": 8, "transcribe": 1}, dict.fromkeys(SYNC_X2, 1)
    ),
    "gpu-12-audio": Profile("lane", "public", {"audio": 8, "lyrics": 8}, {}),
    "gpu-16": Profile(
        "slot", "public", {"audio": 8, "lyrics": 16, "transcribe": 2}, dict.fromkeys(SYNC_X2, 1)
    ),
    "gpu-24": Profile("slot", "trusted", TRUSTED_LANES, SYNC_X2, local_llm=True),
    "gpu-24-lane": Profile("lane", "trusted", TRUSTED_LANES, SYNC_X2, local_llm=True),
    "gpu-48": Profile(
        "slot",
        "trusted",
        {
            "audio": 32,
            "lyrics": 64,
            "transcribe": 8,
            "encode": 64,
            "collab": 1,
            "taste": 1,
            "ai": 64,
        },
        dict.fromkeys(SYNC_X2, 4),
        local_llm=True,
    ),
    "cpu": Profile(
        "lane",
        "trusted",
        {
            "audio": 2,
            "lyrics": 8,
            "transcribe": 2,
            "encode": 16,
            "collab": 1,
            "taste": 1,
            "ai": 16,
        },
        dict.fromkeys(SYNC_X2, 1),
        local_llm=True,
    ),
    "cpu-tts": Profile("lane", "trusted", {"speak": 1}, {}),
    "gpu-tts": Profile("lane", "public", {"speak": 1}, {}),
}
SPEAK_ONLY = {"speak"}


def load(profile: str) -> s.Settings:
    return s.load(CONFIG_DIR, {**BASE_ENV, **LLM_KEYS, s.PROFILE_ENV: profile})


def test_shipped_profiles_are_exactly_the_design_set() -> None:
    shipped = {path.stem for path in (CONFIG_DIR / "profiles").glob("*.toml")}
    assert shipped == set(DESIGN)


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_profile_matches_the_design(name: str) -> None:
    design = DESIGN[name]
    settings = load(name)
    assert settings.runtime.mode == design.mode
    assert settings.worker.trust == design.trust
    assert set(settings.lanes.enabled) == set(design.lanes)
    assert {lane: settings.lanes.capacity[lane] for lane in design.lanes} == design.lanes
    for slot, replicas in design.replicas.items():
        assert settings.slots[slot].replicas == replicas, slot
    assert (s.LOCAL_LLM_SLOT in s.slots_for_lanes(settings)) is design.local_llm


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_profile_ships_every_fallback(name: str) -> None:
    settings = load(name)
    assert s.fallback_gaps(settings) == []
    assert settings.slots["align"].fallback == "mms"
    assert settings.slots["sep"].fallback == s.MIX_FALLBACK
    assert settings.sync.align.region_fallback and settings.sync.align.gap_fill
    assert settings.sync.rescue_strategy == "global_ctc"
    assert settings.runtime.oom_unload is True
    assert settings.llm.fallback and settings.llm.fallback != settings.llm.primary


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_profile_passes_both_public_locks(contract: Contract, name: str) -> None:
    settings = load(name)
    s.check_public_lanes(settings, contract.public_lanes)
    speaks_only = set(settings.lanes.enabled) == SPEAK_ONLY
    assert settings.is_public == bool(settings.small_card) or speaks_only
    if settings.is_public:
        assert set(settings.lanes.enabled) <= contract.public_lanes


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_trusted_hosts_require_every_lane_they_serve(name: str) -> None:
    settings = load(name)
    required = {lane for lane in settings.lanes.enabled if settings.lanes.required[lane]}
    expected = set() if settings.is_public else set(settings.lanes.enabled)
    assert required == expected


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_capacity_fits_the_consumer_and_the_separator(contract: Contract, name: str) -> None:
    settings = load(name)
    for lane in settings.lanes.enabled:
        assert settings.lanes.capacity[lane] <= contract.lane(lane).max_ack_pending, lane
    if "transcribe" in settings.lanes.enabled:
        assert settings.lanes.capacity["transcribe"] <= 2 * settings.slots["sep"].replicas


@pytest.mark.parametrize("name", sorted(DESIGN))
def test_ping_catches_a_dead_socket_before_any_lease_expires(contract: Contract, name: str) -> None:
    settings = load(name)
    ping = settings.nats.ping
    slack = min(
        contract.lane(lane).ack_wait_s - contract.lane(lane).heartbeat_s
        for lane in settings.lanes.enabled
    )
    assert ping.interval_s * (ping.max_outstanding + 1) < slack


@pytest.mark.parametrize("name", ["gpu-12", "gpu-12-audio", "gpu-16"])
def test_small_card_profiles_unload_idle_slots_and_use_their_batches(name: str) -> None:
    settings = load(name)
    batches = s.SMALL_CARD_MAX_BATCH[settings.small_card]
    assert settings.runtime.idle_unload_s > 0
    assert {slot: settings.slots[slot].max_batch for slot in batches} == dict(batches)
    assert s.LOCAL_LLM_SLOT not in s.slots_for_lanes(settings)


def test_sixteen_gigabyte_profile_sits_between_the_twelve_and_the_twenty_four() -> None:
    small, middle, large = load("gpu-12"), load("gpu-16"), load("gpu-24")
    for lane in middle.lanes.enabled:
        capacity = middle.lanes.capacity[lane]
        assert small.lanes.capacity[lane] < capacity < large.lanes.capacity[lane], lane
    for slot in SLOT_MODE_MAX_BATCH:
        batch = middle.slots[slot].max_batch
        assert small.slots[slot].max_batch <= batch <= large.slots[slot].max_batch, slot


def test_sixteen_gigabyte_profile_runs_one_engine_per_model(contract: Contract) -> None:
    blueprint = Blueprint.of(load("gpu-16"), contract, {FASTTEXT_HOME_ENV: "/tmp"}, cpus=16)
    assert sorted(plan.name for plan in blueprint.plans) == [
        "align",
        "asr",
        "cpu-tools",
        "mms",
        "mulan",
        "muq",
        "sep",
        "text",
    ]


@pytest.mark.parametrize("name", ["gpu-24", "gpu-48"])
def test_slot_mode_gpu_profiles_use_batches_that_fit_the_card(name: str) -> None:
    settings = load(name)
    batches = {slot: settings.slots[slot].max_batch for slot in SLOT_MODE_MAX_BATCH}
    assert batches == SLOT_MODE_MAX_BATCH


def test_cpu_profile_serves_every_lane_with_one_copy_of_each_model() -> None:
    settings = load("cpu")
    assert settings.runtime.device == "cpu"
    assert settings.runtime.onednn is False
    assert settings.runtime.allow_slow_lanes is True
    assert settings.runtime.isolate_encode is True
    assert set(settings.lanes.enabled) == set(s.LANES) - SPEAK_ONLY
    assert all(settings.slots[slot].replicas == 1 for slot in SYNC_X2)
    assert all(settings.lanes.capacity[lane] > 1 for lane in ("audio", "lyrics", "transcribe"))


def test_only_the_cpu_profile_isolates_encode() -> None:
    for name in DESIGN:
        assert load(name).runtime.isolate_encode is (name == "cpu"), name


def test_the_local_llm_loads_exactly_where_the_ai_lane_is_served() -> None:
    for name, design in DESIGN.items():
        settings = load(name)
        loads_local = s.LOCAL_LLM_SLOT in s.slots_for_lanes(settings)
        assert loads_local is ("ai" in settings.lanes.enabled) is design.local_llm, name
        assert not (loads_local and settings.is_public), name


@pytest.mark.parametrize("name", sorted(name for name in DESIGN if DESIGN[name].local_llm))
def test_the_local_llm_runs_on_cpu_threads_in_its_own_reserved_engine(
    name: str, contract: Contract
) -> None:
    settings = load(name)
    local = settings.llm.local
    blueprint = Blueprint.of(settings, contract, {FASTTEXT_HOME_ENV: "/tmp"}, cpus=8)
    hosts = [plan for plan in blueprint.plans if s.LOCAL_LLM_SLOT in plan.slot_names]
    assert [plan.slot_names for plan in hosts] == [(s.LOCAL_LLM_SLOT,)]
    plan = hosts[0]
    assert (plan.reserved, plan.nice, plan.threads) == (True, local.nice, local.threads)
    assert 0 < plan.nice < BULK_NICE and plan.shares_budget
    assert plan.idle_unload_s == local.idle_unload_s > 0
    spec = plan.slots[0]
    assert spec.device == s.LOCAL_LLM_DEVICE
    assert not devices.uses_torch(spec.device)
    assert (spec.model, spec.revision) == (
        settings.slots[s.LOCAL_LLM_SLOT].model,
        settings.slots[s.LOCAL_LLM_SLOT].revision,
    )
    assert spec.options == {
        "file": local.file,
        "context_tokens": local.context_tokens,
        "chunk_tokens": local.eval_chunk_tokens,
        "state_cache_mib": local.state_cache_mib,
    }
    assert all(other.idle_unload_s == 0 for other in blueprint.plans if other is not plan)


def test_only_the_speech_profiles_serve_the_speak_lane() -> None:
    for name in DESIGN:
        serves = "speak" in load(name).lanes.enabled
        assert serves is (name in ("cpu-tts", "gpu-tts")), name


def test_the_tts_profile_runs_one_reserved_speech_engine(contract: Contract) -> None:
    settings = load("cpu-tts")
    blueprint = Blueprint.of(settings, contract, {TTS_HOME_ENV: "/models/tts"}, cpus=4)
    assert [lane.name for lane in blueprint.lanes] == ["speak"]
    [plan] = blueprint.plans
    assert plan.slot_names == (s.TTS_SLOT,)
    assert plan.reserved
    assert plan.idle_unload_s == settings.tts.idle_unload_s == 600
    [spec] = plan.slots
    assert spec.device == s.TTS_DEVICES["cpu"]
    assert not devices.uses_torch(spec.device)
    assert spec.options == tts_options(settings.tts)
    assert spec.options["program"] == "qwen-engine-linux-x86_64/2/scd-qwen"
    assert sorted(spec.options["voices"]) == ["loli", "moriarty"]
    assert blueprint.launch.env == {TTS_HOME_ENV: "/models/tts"}


@pytest.mark.parametrize(
    ("logical_cpus", "affinity_cpus", "quota", "engine_threads"),
    [
        (12, 12, None, 6),
        (12, 4, None, 4),
        (12, 2, None, 2),
        (64, 64, 10, 10),
        (32, 32, 15, 15),
        (64, 8, 10, 8),
        (12, 12, 1, 1),
        (3, 3, None, 1),
        (1, 1, None, 1),
        (None, 4, None, 1),
    ],
)
def test_the_tts_plan_gives_the_engine_half_the_cpus_within_the_cpu_mask_and_quota(
    contract: Contract,
    monkeypatch: pytest.MonkeyPatch,
    logical_cpus: int | None,
    affinity_cpus: int,
    quota: int | None,
    engine_threads: int,
) -> None:
    monkeypatch.setattr(os, "cpu_count", lambda: logical_cpus)
    monkeypatch.setattr(app, "available_cpus", lambda: affinity_cpus)
    monkeypatch.setattr(app, "cpu_quota", lambda: quota)
    env = {TTS_HOME_ENV: "/models/tts"}
    [plan] = Blueprint.of(load("cpu-tts"), contract, env, cpus=4).plans

    assert plan.own_threads
    assert plan.threads == engine_threads
    assert plan.env == {TTS_THREADS_ENV: str(engine_threads)}


def test_the_gpu_speech_profile_runs_the_voice_on_cuda_on_a_public_node(
    contract: Contract,
) -> None:
    settings = load("gpu-tts")
    blueprint = Blueprint.of(settings, contract, {TTS_HOME_ENV: "/models/tts"}, cpus=10)

    assert settings.is_public and settings.runtime.device == settings.tts.device == "cuda"
    assert [lane.name for lane in blueprint.lanes] == ["speak"]
    [plan] = blueprint.plans
    [spec] = plan.slots
    assert spec.device == s.TTS_DEVICES["cuda"]
    assert not devices.uses_torch(spec.device)
    assert spec.options == tts_options(settings.tts)
    assert plan.reserved and plan.own_threads
    assert plan.env == {TTS_THREADS_ENV: str(plan.threads)}


def test_the_tts_profile_needs_its_models_folder(contract: Contract) -> None:
    with pytest.raises(StartupError, match=TTS_HOME_ENV):
        Blueprint.of(load("cpu-tts"), contract, {}, cpus=4)
