from __future__ import annotations

import dataclasses
import tomllib
import typing
from collections.abc import Mapping
from pathlib import Path

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from worker import settings as s

WILDCARD = "*"
SHIPPED = s.load(CONFIG_DIR, BASE_ENV)


def schema_paths(cls: type, prefix: tuple[str, ...] = ()) -> set[tuple[str, ...]]:
    paths: set[tuple[str, ...]] = set()
    hints = typing.get_type_hints(cls)
    for field in dataclasses.fields(cls):
        if field.name == "profile":
            continue
        paths |= hint_paths(
            hints[field.name], (*prefix, field.name), field.default is not dataclasses.MISSING
        )
    return paths


def hint_paths(hint: object, path: tuple[str, ...], optional: bool) -> set[tuple[str, ...]]:
    origin = typing.get_origin(hint)
    if origin in (Mapping, dict):
        item = typing.get_args(hint)[1]
        if dataclasses.is_dataclass(item):
            return schema_paths(item, (*path, WILDCARD))
        return {(*path, WILDCARD)}
    if dataclasses.is_dataclass(hint) and isinstance(hint, type):
        return schema_paths(hint, path)
    return set() if optional else {path}


def toml_paths(node: Mapping[str, object], prefix: tuple[str, ...] = ()) -> set[tuple[str, ...]]:
    paths: set[tuple[str, ...]] = set()
    for key, value in node.items():
        if isinstance(value, Mapping):
            paths |= toml_paths(value, (*prefix, key))
        else:
            paths.add((*prefix, key))
    return paths


def normalized(paths: set[tuple[str, ...]], schema: set[tuple[str, ...]]) -> set[tuple[str, ...]]:
    wildcard_prefixes = {path[: path.index(WILDCARD)] for path in schema if WILDCARD in path}
    result = set()
    for path in paths:
        for prefix in wildcard_prefixes:
            if path[: len(prefix)] == prefix and len(path) > len(prefix):
                path = (*prefix, WILDCARD, *path[len(prefix) + 1 :])
                break
        result.add(path)
    return result


def leaf_paths(value: object, prefix: tuple[str, ...] = ()) -> list[tuple[str, ...]]:
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return [
            path
            for field in dataclasses.fields(value)
            for path in leaf_paths(getattr(value, field.name), (*prefix, field.name))
        ]
    return [prefix]


def env_name(path: tuple[str, ...]) -> str:
    return s.ENV_PREFIX + "__".join(part.upper() for part in path)


def test_worker_toml_matches_settings() -> None:
    raw = tomllib.loads((CONFIG_DIR / "worker.toml").read_text(encoding="utf-8"))
    schema = schema_paths(s.Settings)
    shipped = normalized(toml_paths(raw), schema)
    optional = {("slots", WILDCARD, "fallback"), ("slots", WILDCARD, "memory_growth_cap_mib")}
    empty_by_default = {("runtime", "turn_groups", WILDCARD)}
    assert shipped - schema - optional == set(), "keys in worker.toml unknown to settings.py"
    assert schema - shipped - empty_by_default == set(), (
        "settings.py fields missing from worker.toml"
    )


def test_shipped_config_has_all_lanes_and_slots(settings: s.Settings) -> None:
    assert set(settings.lanes.capacity) == set(s.LANES)
    assert set(settings.lanes.required) == set(s.LANES)
    needed = {slot for lane in s.LANES for slot in s.LANE_SLOTS[lane]}
    assert needed <= set(settings.slots)
    for batches in s.SMALL_CARD_MAX_BATCH.values():
        assert set(batches) <= set(settings.slots)
    assert settings.lanes.enabled == (
        "audio",
        "lyrics",
        "transcribe",
        "encode",
        "collab",
        "taste",
        "ai",
    )
    assert settings.lanes.capacity["transcribe"] <= 2 * settings.slots["sep"].replicas


def test_shipped_config_enables_fallbacks(base_env: dict[str, str]) -> None:
    settings = s.load(CONFIG_DIR, base_env)
    assert s.fallback_gaps(settings) == []
    assert settings.slots["align"].fallback == "mms"
    assert settings.slots["sep"].fallback == s.MIX_FALLBACK
    assert settings.sync.rescue_strategy == "global_ctc"
    assert settings.runtime.oom_unload is True


@pytest.mark.parametrize("profile", sorted((CONFIG_DIR / "profiles").glob("*.toml")))
def test_shipped_profiles_enable_fallbacks(profile: Path, base_env: dict[str, str]) -> None:
    settings = s.load(CONFIG_DIR, {**base_env, s.PROFILE_ENV: profile.stem})
    assert s.fallback_gaps(settings) == []
    if settings.is_public:
        public = {"audio", "lyrics", "transcribe", "speak", "video-keyframe", "video-shot"}
        assert set(settings.lanes.enabled) <= public


@pytest.mark.parametrize(
    "profile", ["", *sorted(p.stem for p in (CONFIG_DIR / "profiles").glob("*.toml"))]
)
def test_shipped_configs_enable_the_ai_lane_only_with_the_local_model(
    profile: str, base_env: dict[str, str]
) -> None:
    settings = s.load(CONFIG_DIR, {**base_env, s.PROFILE_ENV: profile})
    if s.AI_LANE in settings.lanes.enabled:
        assert settings.llm.local.enabled is True
        assert s.LOCAL_LLM_SLOT in s.slots_for_lanes(settings)


def test_fallback_gaps_are_reported(base_env: dict[str, str]) -> None:
    env = {
        **base_env,
        "WORKER__SYNC__ALIGN__REGION_FALLBACK": "false",
        "WORKER__SYNC__ALIGN__GAP_FILL": "false",
        "WORKER__SYNC__RESCUE_STRATEGY": "none",
        "WORKER__RUNTIME__OOM_UNLOAD": "false",
    }
    gaps = s.fallback_gaps(s.load(CONFIG_DIR, env))
    assert len(gaps) == 4
    assert any("region_fallback" in gap for gap in gaps)


@pytest.mark.parametrize(("profile", "card"), [("gpu-12-audio", "gpu-12"), ("gpu-16", "gpu-16")])
def test_small_card_gaps_cover_idle_unload_and_batches(
    base_env: dict[str, str], tmp_path: Path, profile: str, card: str
) -> None:
    config = tmp_path / "config"
    config.mkdir()
    (config / "worker.toml").write_bytes((CONFIG_DIR / "worker.toml").read_bytes())
    (config / "profiles").mkdir()
    (config / "profiles" / f"{profile}.toml").write_text(
        '[lanes]\nenabled = ["audio", "lyrics"]\n', encoding="utf-8"
    )
    gaps = s.fallback_gaps(s.load(config, {**base_env, s.PROFILE_ENV: profile}))
    assert f"runtime.idle_unload_s must be > 0 on {card} profiles" in gaps
    assert any(gap.startswith("slots.muq.max_batch") for gap in gaps)


def test_sync_version_has_schema_and_revisions(settings: s.Settings) -> None:
    version = s.sync_version(settings)
    schema, align_rev, mms_rev, sha8 = version.split(".")
    assert schema == "s4"
    assert align_rev == settings.slots["align"].revision[:8]
    assert mms_rev == settings.slots["mms"].revision[:8]
    assert len(sha8) == 8


@pytest.mark.parametrize("path", leaf_paths(SHIPPED.sync, ("sync",)), ids="/".join)
def test_every_sync_key_changes_sync_version(
    base_env: dict[str, str], settings: s.Settings, path: tuple[str, ...]
) -> None:
    current = settings.sync
    for part in path[1:]:
        current = getattr(current, part)
    if isinstance(current, bool):
        changed = str(not current).lower()
    elif isinstance(current, int):
        changed = str(current + 1)
    elif isinstance(current, float):
        changed = str(current + 0.01)
    else:
        options = {
            "strategy": "global_ctc",
            "rescue_strategy": "none",
            "confidence_model": "v1",
        }
        changed = options[path[-1]]
        if changed == current:
            pytest.skip("only one legal value differs by validation path")
    tweaked = s.load(CONFIG_DIR, {**base_env, env_name(path): changed})
    assert s.sync_version(tweaked) != s.sync_version(settings)


@pytest.mark.parametrize("slot", s.SYNC_MODEL_SLOTS)
def test_sync_model_revision_changes_sync_version(
    base_env: dict[str, str], settings: s.Settings, slot: str
) -> None:
    tweaked = s.load(CONFIG_DIR, {**base_env, env_name(("slots", slot, "revision")): "f" * 40})
    assert s.sync_version(tweaked) != s.sync_version(settings)


def test_unrelated_keys_keep_sync_version(base_env: dict[str, str], settings: s.Settings) -> None:
    tweaked = s.load(
        CONFIG_DIR,
        {
            **base_env,
            "WORKER__AUDIO__MAX_DOWNLOAD_MIB": "128",
            "WORKER__SLOTS__MUQ__REVISION": "e" * 40,
            "WORKER__LANES__CAPACITY": "{ audio = 1 }",
        },
    )
    assert s.sync_version(tweaked) == s.sync_version(settings)


def test_model_ref_and_slots_for_lanes(settings: s.Settings) -> None:
    assert s.model_ref(settings, "align") == "Qwen/Qwen3-ForcedAligner-0.6B-hf@c07281df"
    assert s.model_ref(settings, "train-collab") == ""
    assert s.slots_for_lanes(settings) == (
        "muq",
        "mulan",
        "cpu-tools",
        "text",
        "sep",
        "asr",
        "align",
        "mms",
        "train-collab",
        "train-taste",
        "llm-local",
    )
