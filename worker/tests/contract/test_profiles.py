from __future__ import annotations

from dataclasses import dataclass

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR, LLM_KEYS
from worker import settings as s
from worker.contract import Contract

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
    "gpu-24": Profile("slot", "trusted", TRUSTED_LANES, SYNC_X2),
    "gpu-24-lane": Profile("lane", "trusted", TRUSTED_LANES, SYNC_X2),
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
    "cpu": Profile("lane", "trusted", {"lyrics": 1, "encode": 1, "collab": 1, "ai": 16}, {}),
}


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
    assert settings.llm.local.enabled is design.local_llm


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
    assert settings.is_public == settings.is_gpu12
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


@pytest.mark.parametrize("name", ["gpu-12", "gpu-12-audio"])
def test_twelve_gigabyte_profiles_unload_idle_slots_and_use_small_batches(name: str) -> None:
    settings = load(name)
    assert settings.runtime.idle_unload_s > 0
    assert {slot: settings.slots[slot].max_batch for slot in s.GPU12_MAX_BATCH} == dict(
        s.GPU12_MAX_BATCH
    )
    assert s.LOCAL_LLM_SLOT not in s.slots_for_lanes(settings)


@pytest.mark.parametrize("name", ["gpu-24", "gpu-48"])
def test_slot_mode_gpu_profiles_use_batches_that_fit_the_card(name: str) -> None:
    settings = load(name)
    batches = {slot: settings.slots[slot].max_batch for slot in SLOT_MODE_MAX_BATCH}
    assert batches == SLOT_MODE_MAX_BATCH


def test_cpu_profile_turns_onednn_off_and_skips_slow_lanes() -> None:
    settings = load("cpu")
    assert settings.runtime.device == "cpu"
    assert settings.runtime.onednn is False
    assert settings.runtime.allow_slow_lanes is False
    assert not {"audio", "transcribe"} & set(settings.lanes.enabled)


def test_only_the_48_gigabyte_profile_loads_the_local_llm() -> None:
    for name in DESIGN:
        settings = load(name)
        loads_local = s.LOCAL_LLM_SLOT in s.slots_for_lanes(settings)
        assert loads_local is (name == "gpu-48"), name
        if settings.llm.fallback == s.LOCAL_PROVIDER:
            assert settings.llm.local.enabled
