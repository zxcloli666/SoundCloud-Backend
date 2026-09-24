from __future__ import annotations

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR, LLM_KEYS
from worker import settings as s
from worker.app import Blueprint
from worker.bus.consumers import served_lanes
from worker.contract import Contract

PROFILES = sorted(path.stem for path in (CONFIG_DIR / "profiles").glob("*.toml"))
TASTE_PROFILES = {"gpu-24", "gpu-24-lane", "gpu-48"}


def load(profile: str, **overrides: str) -> s.Settings:
    return s.load(CONFIG_DIR, {**BASE_ENV, **LLM_KEYS, s.PROFILE_ENV: profile, **overrides})


@pytest.mark.parametrize("profile", PROFILES)
def test_taste_runs_only_on_the_trusted_gpu_hosts(profile: str) -> None:
    settings = load(profile)

    assert ("taste" in settings.lanes.enabled) is (profile in TASTE_PROFILES)
    if profile in TASTE_PROFILES:
        assert settings.worker.trust == "trusted"
        assert settings.lanes.capacity["taste"] == 1
        assert settings.lanes.required["taste"] is True


def test_the_shipped_gate_and_budget_follow_the_design() -> None:
    taste = load("gpu-24").taste

    assert (taste.min_users, taste.train_budget_s, taste.max_object_mib) == (500, 4320, 1024)
    assert taste.train_budget_s == 0.6 * 7200


def test_a_public_node_cannot_serve_taste(contract: Contract) -> None:
    public = load("cpu", WORKER__WORKER__TRUST="public", WORKER__LANES__ENABLED='["taste"]')

    assert contract.lane("taste").public is False
    with pytest.raises(s.SettingsError, match="non-public lanes \\['taste'\\]"):
        served_lanes(public, contract)


def test_gpu12_profiles_refuse_taste() -> None:
    with pytest.raises(s.SettingsError, match="cannot enable \\['taste'\\]"):
        load("gpu-12", WORKER__LANES__ENABLED='["audio", "taste"]')


def test_the_trainer_slot_uses_the_gpu_when_there_is_one(contract: Contract) -> None:
    blueprint = Blueprint.of(load("gpu-24"), contract, {"FASTTEXT_HOME": "/tmp"})

    spec = blueprint.specs["train-taste"]

    assert spec.loader == "worker.models.taste_trainer:TasteTrainerSlot"
    assert spec.device == "cuda"
    assert "taste" in {lane.name for lane in blueprint.lanes}
