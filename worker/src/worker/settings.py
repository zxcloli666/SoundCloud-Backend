from __future__ import annotations

import hashlib
import json
import re
import tomllib
import types
import typing
from collections.abc import Mapping
from dataclasses import MISSING, asdict, dataclass, fields, is_dataclass
from pathlib import Path
from types import MappingProxyType
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from _typeshed import DataclassInstance

ENV_PREFIX = "WORKER__"
PROFILE_ENV = "WORKER_PROFILE"
ENV_REF_PREFIX = "env:"
SECRET_KEYS = frozenset({"password", "api_key"})
INTEGER = re.compile(r"[+-]?\d+")

LANES = ("audio", "lyrics", "transcribe", "encode", "collab", "taste", "ai")
TRUST_LEVELS = ("trusted", "public")
RUNTIME_MODES = ("slot", "lane")
DEVICES = ("auto", "cuda", "cpu")
SYNC_STRATEGIES = ("anchored", "global_ctc")
RESCUE_STRATEGIES = ("global_ctc", "none")
CONFIDENCE_MODELS = ("v1", "calibrated")
PROVIDER_KINDS = ("anthropic", "openai_compatible")
LOCAL_PROVIDER = "local"
LOCAL_LLM_SLOT = "llm-local"
QUANTIZE_MODES = ("nf4",)
MIX_FALLBACK = "mix"

LANE_SLOTS: Mapping[str, tuple[str, ...]] = MappingProxyType(
    {
        "audio": ("muq", "mulan", "cpu-tools"),
        "lyrics": ("text", "cpu-tools"),
        "transcribe": ("sep", "asr", "align", "mms", "cpu-tools"),
        "encode": ("text", "mulan"),
        "collab": ("train-collab",),
        "taste": ("train-taste",),
        "ai": (),
    }
)
CPU_ONLY_SLOTS = frozenset({"cpu-tools", "train-collab"})
SYNC_MODEL_SLOTS = ("sep", "asr", "align", "mms", "cpu-tools")
SYNC_SCHEMA = 4
GPU12_PROFILE_PREFIX = "gpu-12"
GPU12_MAX_BATCH: Mapping[str, int] = MappingProxyType(
    {"muq": 1, "mulan": 1, "text": 8000, "sep": 1, "asr": 4, "align": 2, "mms": 2}
)


class SettingsError(ValueError):
    pass


@dataclass(frozen=True)
class WorkerSection:
    id: str
    build: str
    trust: str
    work_dir: str
    contract: str


@dataclass(frozen=True)
class PingSettings:
    interval_s: float
    max_outstanding: int


@dataclass(frozen=True)
class OutboxSettings:
    max_results: int
    max_mib: int


@dataclass(frozen=True)
class NatsSection:
    url: str
    user: str
    password: str
    ping: PingSettings
    outbox: OutboxSettings


@dataclass(frozen=True)
class RuntimeSection:
    mode: str
    device: str
    release_after_call: bool
    recycle_after_calls: int
    recycle_gap_mib: int
    idle_unload_s: int
    oom_unload: bool
    onednn: bool
    shutdown_grace_s: int
    allow_slow_lanes: bool


@dataclass(frozen=True)
class LanesSection:
    enabled: tuple[str, ...]
    capacity: Mapping[str, int]
    required: Mapping[str, bool]
    required_grace_s: int


@dataclass(frozen=True)
class SlotSettings:
    model: str
    revision: str
    replicas: int
    max_batch: int
    max_wait_ms: int
    fallback: str | None = None


@dataclass(frozen=True)
class AudioSection:
    download_timeout_s: float
    max_download_mib: int
    min_duration_s: float
    max_duration_s: float
    silence_dbfs: float
    fingerprint_s: int


@dataclass(frozen=True)
class VadSettings:
    threshold: float
    min_speech_ms: int
    min_silence_ms: int
    pad_ms: int
    region_min_s: float
    region_max_s: float


@dataclass(frozen=True)
class AsrSettings:
    rate_cjk: float
    rate_other: float
    token_margin: int
    loop_ngram: int
    loop_share: float
    repetition_penalty: float


@dataclass(frozen=True)
class AlignSettings:
    region_fallback: bool
    region_min_score: float
    gap_fill: bool
    gap_fill_max_s: float


@dataclass(frozen=True)
class AnchorSettings:
    skip_region: float
    skip_line: float
    overflow: float
    words_per_s: float
    cjk_chars_per_s: float


@dataclass(frozen=True)
class ConfidenceV1Weights:
    placed_share: float
    inside_share: float
    collapsed: float
    order: float


@dataclass(frozen=True)
class ConfidenceWeights:
    aligned_share: float
    inside_share: float
    collapsed: float
    anchor: float
    aligner: float
    separated: float
    order: float
    bias: float

    def all_zero(self) -> bool:
        return all(value == 0.0 for value in asdict(self).values())


@dataclass(frozen=True)
class QualitySettings:
    min_aligned_share: float
    min_placed_share: float
    max_interpolated_share: float
    min_confidence: float
    min_anchor_agreement: float
    min_unconfirmed_anchor_agreement: float
    max_out_of_order_share: float
    min_inside_share: float
    max_collapsed_share: float
    max_rate_outliers: float
    mix_penalty: float
    max_interpolation_gap_s: float
    max_words_collapsed_share: float
    confidence_model: str
    confidence_v1_weights: ConfidenceV1Weights
    confidence_weights: ConfidenceWeights


@dataclass(frozen=True)
class SyncSection:
    strategy: str
    rescue_strategy: str
    unsupported_romanization_share: float
    window_pad_s: float
    lrc_pause_s: float
    vad: VadSettings
    asr: AsrSettings
    align: AlignSettings
    anchors: AnchorSettings
    quality: QualitySettings


@dataclass(frozen=True)
class CollabSection:
    max_object_mib: int


@dataclass(frozen=True)
class TasteSection:
    max_object_mib: int
    min_users: int
    train_budget_s: int


@dataclass(frozen=True)
class ProviderSettings:
    kind: str
    endpoint: str
    model: str
    api_key: str
    soft_timeout_s: float
    hard_timeout_s: float

    @property
    def enabled(self) -> bool:
        return bool(self.api_key)


@dataclass(frozen=True)
class BreakerSettings:
    failures: int
    window_s: float
    open_s: float


@dataclass(frozen=True)
class LocalLlmSettings:
    enabled: bool
    quantize: str


@dataclass(frozen=True)
class LlmSection:
    primary: str
    fallback: str
    hedge_after_share: float
    breaker: BreakerSettings
    providers: Mapping[str, ProviderSettings]
    local: LocalLlmSettings


@dataclass(frozen=True)
class Settings:
    worker: WorkerSection
    nats: NatsSection
    runtime: RuntimeSection
    lanes: LanesSection
    slots: Mapping[str, SlotSettings]
    audio: AudioSection
    sync: SyncSection
    collab: CollabSection
    taste: TasteSection
    llm: LlmSection
    profile: str = ""

    @property
    def is_public(self) -> bool:
        return self.worker.trust == "public"

    @property
    def is_gpu12(self) -> bool:
        return self.profile.startswith(GPU12_PROFILE_PREFIX)


def load(config_dir: Path, environ: Mapping[str, str]) -> Settings:
    layered = read_toml(config_dir / "worker.toml")
    profile = environ.get(PROFILE_ENV, "")
    if profile:
        profile_path = config_dir / "profiles" / f"{profile}.toml"
        if not profile_path.is_file():
            raise SettingsError(f"profile file not found: {profile_path}")
        layered = overlay(layered, read_toml(profile_path))
    layered = overlay(layered, env_overrides(environ, layered))
    return build(layered, profile, environ)


def build(raw: Mapping[str, object], profile: str, environ: Mapping[str, str]) -> Settings:
    check_secret_refs(raw, ())
    resolved = resolve_env_refs(raw, environ)
    settings = construct(Settings, {**resolved, "profile": profile}, ())
    validate(settings)
    return settings


def read_toml(path: Path) -> dict[str, object]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def overlay(base: Mapping[str, object], extra: Mapping[str, object]) -> dict[str, object]:
    merged = dict(base)
    for key, value in extra.items():
        current = merged.get(key)
        if isinstance(current, Mapping) and isinstance(value, Mapping):
            merged[key] = overlay(current, value)
        else:
            merged[key] = value
    return merged


def env_overrides(environ: Mapping[str, str], layered: Mapping[str, object]) -> dict[str, object]:
    overrides: dict[str, object] = {}
    for name, raw in environ.items():
        if not name.startswith(ENV_PREFIX):
            continue
        segments = name[len(ENV_PREFIX) :].split("__")
        path = resolve_path(layered, segments)
        current = value_at(layered, path)
        set_at(overrides, path, coerce(raw, current, name))
    return overrides


def resolve_path(layered: Mapping[str, object], segments: list[str]) -> tuple[str, ...]:
    path: list[str] = []
    node: object = layered
    for segment in segments:
        key = segment.lower()
        if isinstance(node, Mapping):
            matches = [k for k in node if k.lower().replace("-", "_") == key]
            if matches:
                key = matches[0]
        path.append(key)
        node = node.get(key) if isinstance(node, Mapping) else None
    return tuple(path)


def value_at(tree: Mapping[str, object], path: tuple[str, ...]) -> object:
    node: object = tree
    for key in path:
        if not isinstance(node, Mapping) or key not in node:
            return None
        node = node[key]
    return node


def set_at(tree: dict[str, object], path: tuple[str, ...], value: object) -> None:
    node = tree
    for key in path[:-1]:
        child = node.get(key)
        if not isinstance(child, dict):
            child = {}
            node[key] = child
        node = child
    node[path[-1]] = value


def coerce(raw: str, current: object, name: str) -> object:
    if isinstance(current, bool):
        lowered = raw.strip().lower()
        if lowered in ("true", "1", "yes"):
            return True
        if lowered in ("false", "0", "no"):
            return False
        raise SettingsError(f"{name}: expected a boolean, got {raw!r}")
    if isinstance(current, int | float):
        if INTEGER.fullmatch(raw.strip()):
            return int(raw)
        try:
            return float(raw)
        except ValueError as error:
            raise SettingsError(f"{name}: expected a number, got {raw!r}") from error
    if isinstance(current, list | Mapping):
        try:
            return tomllib.loads(f"value = {raw}")["value"]
        except tomllib.TOMLDecodeError as error:
            raise SettingsError(f"{name}: expected a TOML value, got {raw!r}") from error
    return raw


def check_secret_refs(node: Mapping[str, object], path: tuple[str, ...]) -> None:
    for key, value in node.items():
        if isinstance(value, Mapping):
            check_secret_refs(value, (*path, key))
        elif key in SECRET_KEYS and not (isinstance(value, str) and is_env_ref(value)):
            raise SettingsError(f"{dotted((*path, key))}: secrets are given only as 'env:NAME'")


def is_env_ref(value: str) -> bool:
    return value.startswith(ENV_REF_PREFIX)


def resolve_env_refs(node: Mapping[str, object], environ: Mapping[str, str]) -> dict[str, object]:
    resolved: dict[str, object] = {}
    for key, value in node.items():
        if isinstance(value, Mapping):
            resolved[key] = resolve_env_refs(value, environ)
        elif isinstance(value, str) and is_env_ref(value):
            resolved[key] = environ.get(value[len(ENV_REF_PREFIX) :], "")
        else:
            resolved[key] = value
    return resolved


def construct[T: DataclassInstance](cls: type[T], raw: object, path: tuple[str, ...]) -> T:
    if not isinstance(raw, Mapping):
        raise SettingsError(f"{dotted(path) or 'root'}: expected a table")
    hints = typing.get_type_hints(cls)
    known = {field.name for field in fields(cls)}
    unknown = sorted(set(raw) - known)
    if unknown:
        raise SettingsError(f"{dotted(path) or 'root'}: unknown keys {unknown}")
    values: dict[str, object] = {}
    for field in fields(cls):
        field_path = (*path, field.name)
        if field.name not in raw:
            if field.default is not MISSING:
                continue
            raise SettingsError(f"{dotted(field_path)}: missing")
        values[field.name] = convert(hints[field.name], raw[field.name], field_path)
    return cls(**values)


def convert(hint: object, value: object, path: tuple[str, ...]) -> object:
    origin = typing.get_origin(hint)
    if origin is types.UnionType:
        options = [option for option in typing.get_args(hint) if option is not type(None)]
        if value is None:
            return None
        return convert(options[0], value, path)
    if is_dataclass(hint) and isinstance(hint, type):
        return construct(hint, value, path)
    if origin is tuple:
        if not isinstance(value, list):
            raise SettingsError(f"{dotted(path)}: expected an array")
        item_type = typing.get_args(hint)[0]
        return tuple(convert(item_type, item, (*path, str(i))) for i, item in enumerate(value))
    if origin in (Mapping, dict):
        if not isinstance(value, Mapping):
            raise SettingsError(f"{dotted(path)}: expected a table")
        item_type = typing.get_args(hint)[1]
        return MappingProxyType(
            {key: convert(item_type, item, (*path, key)) for key, item in value.items()}
        )
    return scalar(hint, value, path)


def scalar(hint: object, value: object, path: tuple[str, ...]) -> object:
    if hint is bool:
        if isinstance(value, bool):
            return value
        raise SettingsError(f"{dotted(path)}: expected a boolean")
    if hint is int:
        if isinstance(value, int) and not isinstance(value, bool):
            return value
        raise SettingsError(f"{dotted(path)}: expected an integer")
    if hint is float:
        if isinstance(value, int | float) and not isinstance(value, bool):
            return float(value)
        raise SettingsError(f"{dotted(path)}: expected a number")
    if hint is str:
        if isinstance(value, str):
            return value
        raise SettingsError(f"{dotted(path)}: expected a string")
    raise SettingsError(f"{dotted(path)}: unsupported field type {hint}")


def dotted(path: tuple[str, ...]) -> str:
    return ".".join(path)


def validate(settings: Settings) -> None:
    problems = [
        *validate_worker(settings),
        *validate_nats(settings),
        *validate_runtime(settings),
        *validate_lanes(settings),
        *validate_slots(settings),
        *validate_sync(settings),
        *validate_taste(settings),
        *validate_llm(settings),
    ]
    if problems:
        raise SettingsError("; ".join(problems))


def validate_worker(settings: Settings) -> list[str]:
    worker = settings.worker
    problems = []
    if not worker.id:
        problems.append("worker.id is empty (WORKER_NODE_NAME)")
    if worker.trust not in TRUST_LEVELS:
        problems.append(f"worker.trust must be one of {TRUST_LEVELS}")
    if not worker.work_dir:
        problems.append("worker.work_dir is empty")
    if not worker.contract:
        problems.append("worker.contract is empty")
    return problems


def validate_nats(settings: Settings) -> list[str]:
    nats = settings.nats
    problems = []
    if not nats.url:
        problems.append("nats.url is empty (NATS_URL)")
    if nats.ping.interval_s <= 0 or nats.ping.max_outstanding < 1:
        problems.append("nats.ping must have interval_s > 0 and max_outstanding >= 1")
    if nats.outbox.max_results < 1 or nats.outbox.max_mib < 1:
        problems.append("nats.outbox limits must be >= 1")
    return problems


def validate_runtime(settings: Settings) -> list[str]:
    runtime = settings.runtime
    problems = []
    if runtime.mode not in RUNTIME_MODES:
        problems.append(f"runtime.mode must be one of {RUNTIME_MODES}")
    if runtime.device not in DEVICES:
        problems.append(f"runtime.device must be one of {DEVICES}")
    if runtime.shutdown_grace_s <= 0:
        problems.append("runtime.shutdown_grace_s must be > 0")
    if runtime.idle_unload_s < 0 or runtime.recycle_after_calls < 1 or runtime.recycle_gap_mib < 1:
        problems.append("runtime recycle/idle values out of range")
    return problems


def validate_lanes(settings: Settings) -> list[str]:
    lanes = settings.lanes
    problems = []
    unknown = sorted(set(lanes.enabled) - set(LANES))
    if unknown:
        problems.append(f"lanes.enabled has unknown lanes {unknown}")
    if len(set(lanes.enabled)) != len(lanes.enabled):
        problems.append("lanes.enabled has duplicates")
    for table_name, table in (("capacity", lanes.capacity), ("required", lanes.required)):
        stray = sorted(set(table) - set(LANES))
        if stray:
            problems.append(f"lanes.{table_name} has unknown lanes {stray}")
        missing = [lane for lane in lanes.enabled if lane not in table]
        if missing:
            problems.append(f"lanes.{table_name} lacks enabled lanes {missing}")
    for lane in lanes.enabled:
        if lanes.capacity.get(lane, 0) < 1:
            problems.append(f"lanes.capacity.{lane} must be >= 1")
    if lanes.required_grace_s < 0:
        problems.append("lanes.required_grace_s must be >= 0")
    if settings.is_gpu12:
        heavy = [lane for lane in lanes.enabled if lane not in ("audio", "lyrics", "transcribe")]
        if heavy:
            problems.append(f"profile {settings.profile} cannot enable {heavy}")
    return problems


def validate_slots(settings: Settings) -> list[str]:
    problems = []
    for name, slot in settings.slots.items():
        if slot.replicas < 0 or slot.max_batch < 1 or slot.max_wait_ms < 0:
            problems.append(f"slots.{name}: replicas >= 0, max_batch >= 1, max_wait_ms >= 0")
        allowed_fallbacks = set(settings.slots) | ({MIX_FALLBACK} if name == "sep" else set())
        if slot.fallback is not None and slot.fallback not in allowed_fallbacks:
            problems.append(f"slots.{name}.fallback refers to unknown slot {slot.fallback!r}")
    for lane in settings.lanes.enabled:
        for slot_name in lane_slots(settings, lane):
            needed = settings.slots.get(slot_name)
            if needed is None:
                problems.append(f"lane {lane} needs slots.{slot_name}")
            elif needed.replicas < 1:
                problems.append(f"lane {lane} needs slots.{slot_name}.replicas >= 1")
    return problems


def validate_sync(settings: Settings) -> list[str]:
    sync = settings.sync
    problems = []
    if sync.strategy not in SYNC_STRATEGIES:
        problems.append(f"sync.strategy must be one of {SYNC_STRATEGIES}")
    if sync.rescue_strategy not in RESCUE_STRATEGIES:
        problems.append(f"sync.rescue_strategy must be one of {RESCUE_STRATEGIES}")
    if sync.quality.confidence_model not in CONFIDENCE_MODELS:
        problems.append(f"sync.quality.confidence_model must be one of {CONFIDENCE_MODELS}")
    if sync.quality.confidence_model == "calibrated" and sync.quality.confidence_weights.all_zero():
        problems.append(
            "sync.quality.confidence_model=calibrated needs non-zero confidence_weights"
        )
    if not 0 < sync.unsupported_romanization_share <= 1:
        problems.append("sync.unsupported_romanization_share must be in (0, 1]")
    if sync.vad.region_min_s <= 0 or sync.vad.region_max_s <= sync.vad.region_min_s:
        problems.append("sync.vad regions must satisfy 0 < region_min_s < region_max_s")
    return problems


def validate_taste(settings: Settings) -> list[str]:
    taste = settings.taste
    if taste.max_object_mib < 1 or taste.min_users < 1 or taste.train_budget_s < 1:
        return ["taste.max_object_mib, min_users and train_budget_s must be >= 1"]
    return []


def validate_llm(settings: Settings) -> list[str]:
    llm = settings.llm
    problems = []
    if LOCAL_PROVIDER in llm.providers:
        problems.append(f"llm.providers.{LOCAL_PROVIDER} is reserved for [llm.local]")
    known = set(llm.providers) | ({LOCAL_PROVIDER} if llm.local.enabled else set())
    for role, name in (("primary", llm.primary), ("fallback", llm.fallback)):
        if name and name not in known:
            problems.append(f"llm.{role} refers to unknown provider {name!r}")
    if llm.primary and llm.primary == llm.fallback:
        problems.append("llm.primary and llm.fallback must differ")
    if not 0 < llm.hedge_after_share <= 1:
        problems.append("llm.hedge_after_share must be in (0, 1]")
    for name, provider in llm.providers.items():
        if provider.kind not in PROVIDER_KINDS:
            problems.append(f"llm.providers.{name}.kind must be one of {PROVIDER_KINDS}")
        if provider.soft_timeout_s <= 0 or provider.hard_timeout_s < provider.soft_timeout_s:
            problems.append(f"llm.providers.{name}: 0 < soft_timeout_s <= hard_timeout_s")
        if provider.enabled and not (provider.endpoint and provider.model):
            problems.append(f"llm.providers.{name} has an api_key but no endpoint or model")
    if llm.breaker.failures < 1 or llm.breaker.window_s <= 0 or llm.breaker.open_s <= 0:
        problems.append("llm.breaker values out of range")
    if llm.local.quantize not in QUANTIZE_MODES:
        problems.append(f"llm.local.quantize must be one of {QUANTIZE_MODES}")
    return problems


def check_public_lanes(settings: Settings, public_lanes: frozenset[str]) -> None:
    if not settings.is_public:
        return
    closed = [lane for lane in settings.lanes.enabled if lane not in public_lanes]
    if closed:
        raise SettingsError(f"trust=public cannot enable non-public lanes {closed}")


def fallback_gaps(settings: Settings) -> list[str]:
    gaps = []
    align = settings.slots.get("align")
    if align is None or align.fallback != "mms":
        gaps.append("slots.align.fallback != mms")
    sep = settings.slots.get("sep")
    if sep is None or sep.fallback != MIX_FALLBACK:
        gaps.append("slots.sep.fallback != mix")
    if not settings.sync.align.region_fallback:
        gaps.append("sync.align.region_fallback disabled")
    if not settings.sync.align.gap_fill:
        gaps.append("sync.align.gap_fill disabled")
    if settings.sync.rescue_strategy == "none":
        gaps.append("sync.rescue_strategy disabled")
    if not settings.runtime.oom_unload:
        gaps.append("runtime.oom_unload disabled")
    if "ai" in settings.lanes.enabled:
        gaps += llm_gaps(settings.llm)
    if settings.is_gpu12:
        if settings.runtime.idle_unload_s <= 0:
            gaps.append("runtime.idle_unload_s must be > 0 on gpu-12 profiles")
        for slot_name, max_batch in GPU12_MAX_BATCH.items():
            slot = settings.slots.get(slot_name)
            if slot is not None and slot.max_batch != max_batch:
                gaps.append(f"slots.{slot_name}.max_batch != {max_batch} on gpu-12 profiles")
    return gaps


def llm_gaps(llm: LlmSection) -> list[str]:
    gaps = []
    for role, name in (("primary", llm.primary), ("fallback", llm.fallback)):
        if not name:
            gaps.append(f"llm.{role} empty")
        elif not provider_enabled(llm, name):
            gaps.append(f"llm.{role} provider {name} disabled")
    return gaps


def provider_enabled(llm: LlmSection, name: str) -> bool:
    if name == LOCAL_PROVIDER:
        return llm.local.enabled
    provider = llm.providers.get(name)
    return provider is not None and provider.enabled


def sync_version(settings: Settings) -> str:
    payload = {
        "sync": asdict(settings.sync),
        "models": {name: settings.slots[name].revision for name in SYNC_MODEL_SLOTS},
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    sha8 = hashlib.sha256(canonical).hexdigest()[:8]
    return (
        f"s{SYNC_SCHEMA}."
        f"{settings.slots['align'].revision[:8]}."
        f"{settings.slots['mms'].revision[:8]}."
        f"{sha8}"
    )


def model_ref(settings: Settings, slot: str) -> str:
    spec = settings.slots[slot]
    return f"{spec.model}@{spec.revision[:8]}" if spec.revision else spec.model


def slots_for_lanes(settings: Settings) -> tuple[str, ...]:
    wanted: list[str] = []
    for lane in settings.lanes.enabled:
        for slot in lane_slots(settings, lane):
            if slot not in wanted:
                wanted.append(slot)
    for slot in list(wanted):
        fallback = settings.slots[slot].fallback
        if fallback not in (None, MIX_FALLBACK) and fallback not in wanted:
            wanted.append(fallback)
    return tuple(wanted)


def lane_slots(settings: Settings, lane: str) -> tuple[str, ...]:
    slots = LANE_SLOTS.get(lane, ())
    if lane == "ai" and settings.llm.local.enabled:
        return (*slots, LOCAL_LLM_SLOT)
    return slots
