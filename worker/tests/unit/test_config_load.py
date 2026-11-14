from __future__ import annotations

import ast
import shutil
from pathlib import Path

import pytest

from tests.conftest import CONFIG_DIR
from worker import settings as s

LOCAL_LLM_REF = "unsloth/Qwen3.5-2B-GGUF/Qwen3.5-2B-Q4_K_M.gguf@f6d5376b"


@pytest.fixture
def config_copy(tmp_path: Path) -> Path:
    target = tmp_path / "config"
    shutil.copytree(CONFIG_DIR, target)
    (target / "profiles").mkdir(exist_ok=True)
    return target


def write_profile(config_dir: Path, name: str, body: str) -> None:
    (config_dir / "profiles" / f"{name}.toml").write_text(body, encoding="utf-8")


def test_base_config_loads_with_env_refs_resolved(base_env: dict[str, str]) -> None:
    settings = s.load(CONFIG_DIR, base_env)
    assert settings.worker.id == "test-worker"
    assert settings.nats.url == "nats://127.0.0.1:4222"
    assert settings.nats.password == "secret"
    assert settings.profile == ""
    assert settings.llm.providers["anthropic"].enabled is False


def test_provider_is_enabled_exactly_when_its_key_resolves(base_env: dict[str, str]) -> None:
    env = {**base_env, "ANTHROPIC_API_KEY": "k"}
    settings = s.load(CONFIG_DIR, env)
    assert settings.llm.providers["anthropic"].enabled is True
    assert settings.llm.providers["openai_compatible"].enabled is False
    env = {**env, "LLM_FALLBACK_KEY": "k2", "LLM_FALLBACK_URL": "https://x/v1"}
    env = {**env, "LLM_FALLBACK_MODEL": "m"}
    assert s.load(CONFIG_DIR, env).llm.providers["openai_compatible"].enabled is True


@pytest.mark.parametrize("missing", ["LLM_FALLBACK_URL", "LLM_FALLBACK_MODEL"])
def test_provider_key_without_endpoint_or_model_is_a_start_error(
    base_env: dict[str, str], missing: str
) -> None:
    env = {
        **base_env,
        "LLM_FALLBACK_KEY": "k2",
        "LLM_FALLBACK_URL": "https://x/v1",
        "LLM_FALLBACK_MODEL": "m",
    }
    del env[missing]
    with pytest.raises(s.SettingsError, match="openai_compatible has an api_key but no endpoint"):
        s.load(CONFIG_DIR, env)


def test_the_ai_lane_brings_the_local_llm_slot(base_env: dict[str, str]) -> None:
    settings = s.load(CONFIG_DIR, base_env)
    local = settings.llm.local
    assert s.LOCAL_LLM_SLOT in s.slots_for_lanes(settings)
    assert s.local_llm_ref(settings) == LOCAL_LLM_REF
    assert local.file.endswith(".gguf") and len(local.sha256) == 64
    assert local.enabled is True
    assert (local.context_tokens, local.eval_chunk_tokens, local.idle_unload_s) == (8192, 128, 600)
    assert local.state_cache_mib == 256
    assert (local.threads, local.nice, local.max_in_flight) == (2, 5, 2)
    without = s.load(CONFIG_DIR, {**base_env, "WORKER__LANES__ENABLED": '["audio", "lyrics"]'})
    assert s.LOCAL_LLM_SLOT not in s.slots_for_lanes(without)


def test_the_local_model_can_be_switched_off_without_dropping_the_ai_lane(
    base_env: dict[str, str],
) -> None:
    env = {
        **base_env,
        "WORKER__LLM__LOCAL__ENABLED": "false",
        "WORKER__SLOTS__LLM_LOCAL__REPLICAS": "0",
        "WORKER__SLOTS__LLM_LOCAL__MODEL": "",
    }
    settings = s.load(CONFIG_DIR, env)
    assert settings.llm.local.enabled is False
    assert "ai" in settings.lanes.enabled
    assert s.LOCAL_LLM_SLOT not in s.slots_for_lanes(settings)


def test_the_refiner_chain_cannot_name_the_local_model(base_env: dict[str, str]) -> None:
    with pytest.raises(s.SettingsError, match=r"llm\.fallback refers to unknown provider 'local'"):
        s.load(CONFIG_DIR, {**base_env, "WORKER__LLM__FALLBACK": "local"})


@pytest.mark.parametrize(
    ("override", "problem"),
    [
        ({"WORKER__SLOTS__LLM_LOCAL__REPLICAS": "0"}, r"slots\.llm-local\.replicas >= 1"),
        ({"WORKER__SLOTS__LLM_LOCAL__MODEL": ""}, r"lane ai needs slots\.llm-local\.model"),
        ({"WORKER__SLOTS__LLM_LOCAL__REVISION": ""}, r"lane ai needs slots\.llm-local\.model"),
        ({"WORKER__LLM__LOCAL__FILE": "model.safetensors"}, r"llm\.local\.file must be a \.gguf"),
        ({"WORKER__LLM__LOCAL__SHA256": "ABC"}, r"llm\.local\.sha256 must be 64 lowercase hex"),
        ({"WORKER__LLM__LOCAL__CONTEXT_TOKENS": "512"}, r"llm\.local\.context_tokens must be"),
        ({"WORKER__LLM__LOCAL__EVAL_CHUNK_TOKENS": "0"}, r"llm\.local\.eval_chunk_tokens must be"),
        (
            {"WORKER__LLM__LOCAL__EVAL_CHUNK_TOKENS": "8193"},
            r"llm\.local\.eval_chunk_tokens must be within 1\.\.llm\.local\.context_tokens",
        ),
        ({"WORKER__LLM__LOCAL__STATE_CACHE_MIB": "0"}, r"llm\.local\.state_cache_mib must be"),
        ({"WORKER__LLM__LOCAL__IDLE_UNLOAD_S": "-1"}, r"llm\.local\.idle_unload_s must be >= 0"),
        ({"WORKER__LLM__LOCAL__THREADS": "0"}, r"llm\.local\.threads must be >= 1"),
        ({"WORKER__LLM__LOCAL__MAX_IN_FLIGHT": "0"}, r"llm\.local\.max_in_flight must be"),
        ({"WORKER__LLM__LOCAL__NICE": "20"}, r"llm\.local\.nice must be within 0\.\.19"),
    ],
)
def test_local_llm_settings_are_checked_at_start(
    base_env: dict[str, str], override: dict[str, str], problem: str
) -> None:
    with pytest.raises(s.SettingsError, match=problem):
        s.load(CONFIG_DIR, {**base_env, **override})


def test_a_chunk_as_long_as_the_context_is_accepted(base_env: dict[str, str]) -> None:
    whole = s.load(CONFIG_DIR, {**base_env, "WORKER__LLM__LOCAL__EVAL_CHUNK_TOKENS": "8192"})

    assert whole.llm.local.eval_chunk_tokens == whole.llm.local.context_tokens


def test_the_speech_models_addresses_follow_the_host_in_their_order(
    base_env: dict[str, str],
) -> None:
    shipped = s.load(CONFIG_DIR, base_env)
    moved = s.load(
        CONFIG_DIR,
        {
            **base_env,
            "WORKER__TTS__BASE_URLS": '["https://models.example/voice", "https://mirror.example/v"]',
        },
    )

    assert shipped.tts.base_urls == ("https://storage.scnative.dev/voice-models",)
    assert moved.tts.base_urls == ("https://models.example/voice", "https://mirror.example/v")


def test_the_old_single_address_key_is_a_start_error(base_env: dict[str, str]) -> None:
    with pytest.raises(s.SettingsError, match="base_url"):
        s.load(CONFIG_DIR, {**base_env, "WORKER__TTS__BASE_URL": "https://models.example/voice"})


@pytest.mark.parametrize(
    ("override", "problem"),
    [
        ({"WORKER__TTS__BASE_URLS": '["http://models.example/voice"]'}, r"tts\.base_urls must be"),
        ({"WORKER__TTS__BASE_URLS": '["https://models.example/voice/"]'}, "trailing slash"),
        ({"WORKER__TTS__BASE_URLS": "[]"}, "at least one address"),
        ({"WORKER__TTS__BASE_URLS": '["https://a.example/v", "https://a.example/v"]'}, "each once"),
        ({"WORKER__TTS__DEVICE": "vulkan"}, r"tts\.device must be one of"),
        ({"WORKER__TTS__DOWNLOAD_MIB_S": "-1"}, r"tts\.download_mib_s must be >= 0"),
        (
            {"WORKER__TTS__ENGINE__CUDA_FILES": '{ "../libggml-cuda.so" = "' + "0" * 64 + '" }'},
            "not a plain relative path",
        ),
        ({"WORKER__TTS__ENGINE__CUDA_FILES": '{ "libggml-cuda.so" = "ABC" }'}, "tts sha256 values"),
        ({"WORKER__TTS__IDLE_UNLOAD_S": "-1"}, r"tts\.idle_unload_s must be >= 0"),
        ({"WORKER__TTS__ENGINE__PROGRAM": "qwen"}, r"tts\.engine\.program must be one of"),
        ({"WORKER__TTS__CODEC__PATH": "../codec.gguf"}, "not a plain relative path"),
        ({"WORKER__TTS__CODEC__PATH": "/models/codec.gguf"}, "not a plain relative path"),
        ({"WORKER__TTS__CODEC__SHA256": "ABC"}, "tts sha256 values must be 64 lowercase hex"),
        ({"WORKER__TTS__VOICES__LOLI__SPEAKER": "Loli"}, r"tts\.voices\.loli\.speaker"),
    ],
)
def test_speech_settings_are_checked_at_start(
    base_env: dict[str, str], override: dict[str, str], problem: str
) -> None:
    with pytest.raises(s.SettingsError, match=problem):
        s.load(CONFIG_DIR, {**base_env, **override})


def test_a_config_without_voices_is_a_start_error(
    base_env: dict[str, str], config_copy: Path
) -> None:
    worker_toml = config_copy / "worker.toml"
    shipped = worker_toml.read_text(encoding="utf-8")
    worker_toml.write_text(shipped.split("[tts.voices.")[0] + "[tts.voices]\n", encoding="utf-8")
    with pytest.raises(s.SettingsError, match=r"tts\.voices is empty"):
        s.load(config_copy, base_env)


def test_missing_required_env_ref_is_a_start_error(base_env: dict[str, str]) -> None:
    env = dict(base_env)
    del env["WORKER_NODE_NAME"]
    with pytest.raises(s.SettingsError, match=r"worker\.id"):
        s.load(CONFIG_DIR, env)
    env = dict(base_env)
    del env["NATS_URL"]
    with pytest.raises(s.SettingsError, match=r"nats\.url"):
        s.load(CONFIG_DIR, env)


def test_profile_overlays_base(config_copy: Path, base_env: dict[str, str]) -> None:
    write_profile(
        config_copy,
        "gpu-12",
        """
[worker]
trust = "public"
[runtime]
mode = "lane"
idle_unload_s = 600
[lanes]
enabled = ["audio", "lyrics", "transcribe"]
capacity = { audio = 4, lyrics = 8, transcribe = 1 }
required = { audio = false, lyrics = false, transcribe = false }
[slots.muq]
max_batch = 1
""",
    )
    settings = s.load(config_copy, {**base_env, "WORKER_PROFILE": "gpu-12"})
    assert settings.profile == "gpu-12"
    assert settings.small_card == "gpu-12" and settings.is_public
    assert settings.runtime.mode == "lane"
    assert settings.lanes.enabled == ("audio", "lyrics", "transcribe")
    assert settings.lanes.capacity["audio"] == 4
    assert settings.lanes.capacity["collab"] == 1
    assert settings.slots["muq"].max_batch == 1
    assert settings.slots["muq"].model == "OpenMuQ/MuQ-large-msd-iter"


def test_missing_profile_file_is_a_start_error(config_copy: Path, base_env: dict[str, str]) -> None:
    with pytest.raises(s.SettingsError, match="profile file not found"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "gpu-99"})


def test_env_overrides_are_coerced_by_field_type(base_env: dict[str, str]) -> None:
    env = {
        **base_env,
        "WORKER__RUNTIME__ONEDNN": "false",
        "WORKER__RUNTIME__IDLE_UNLOAD_S": "600",
        "WORKER__SYNC__QUALITY__MIN_CONFIDENCE": "0.7",
        "WORKER__LANES__ENABLED": '["audio", "lyrics"]',
        "WORKER__LANES__CAPACITY": "{ audio = 2, lyrics = 3 }",
        "WORKER__SLOTS__CPU_TOOLS__MAX_BATCH": "4",
        "WORKER__WORKER__BUILD": "2026.09.1+abc123",
    }
    settings = s.load(CONFIG_DIR, env)
    assert settings.runtime.onednn is False
    assert settings.runtime.idle_unload_s == 600
    assert settings.sync.quality.min_confidence == 0.7
    assert settings.lanes.enabled == ("audio", "lyrics")
    assert settings.lanes.capacity["audio"] == 2
    assert settings.lanes.capacity["lyrics"] == 3
    assert settings.lanes.capacity["encode"] == 32
    assert settings.slots["cpu-tools"].max_batch == 4
    assert settings.worker.build == "2026.09.1+abc123"


@pytest.mark.parametrize(
    ("name", "value", "message"),
    [
        ("WORKER__RUNTIME__ONEDNN", "maybe", "expected a boolean"),
        ("WORKER__RUNTIME__IDLE_UNLOAD_S", "soon", "expected a number"),
        ("WORKER__RUNTIME__IDLE_UNLOAD_S", "1.5", "expected an integer"),
        ("WORKER__SYNC__WINDOW_PAD_S", "wide", "expected a number"),
        ("WORKER__LANES__ENABLED", "audio, lyrics", "expected a TOML value"),
        ("WORKER__RUNTIME__THREADS", "many", "expected a number"),
        ("WORKER__RUNTIME__THREADS", "-1", "runtime.threads must be >= 0"),
    ],
)
def test_bad_env_values_are_start_errors(
    base_env: dict[str, str], name: str, value: str, message: str
) -> None:
    with pytest.raises(s.SettingsError, match=message):
        s.load(CONFIG_DIR, {**base_env, name: value})


def test_runtime_threads_come_from_the_environment(base_env: dict[str, str]) -> None:
    assert s.load(CONFIG_DIR, base_env).runtime.threads == 0
    assert s.load(CONFIG_DIR, {**base_env, "WORKER__RUNTIME__THREADS": "6"}).runtime.threads == 6


def test_unknown_key_is_a_start_error(base_env: dict[str, str], config_copy: Path) -> None:
    with pytest.raises(s.SettingsError, match="unknown keys \\['concurrency'\\]"):
        s.load(CONFIG_DIR, {**base_env, "WORKER__LANES__CONCURRENCY": "4"})
    write_profile(config_copy, "odd", "[sync]\nwindow_pad_seconds = 2.0\n")
    with pytest.raises(s.SettingsError, match="sync: unknown keys"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "odd"})


def test_missing_key_is_a_start_error(base_env: dict[str, str], config_copy: Path) -> None:
    write_profile(config_copy, "half", '[slots.extra]\nmodel = "x"\n')
    with pytest.raises(s.SettingsError, match=r"slots\.extra\.revision: missing"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "half"})


def test_wrong_type_is_a_start_error(base_env: dict[str, str], config_copy: Path) -> None:
    write_profile(config_copy, "typed", "[runtime]\nrecycle_after_calls = 1.5\n")
    with pytest.raises(s.SettingsError, match=r"runtime\.recycle_after_calls: expected an integer"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "typed"})


def test_secret_without_env_ref_is_a_start_error(
    base_env: dict[str, str], config_copy: Path
) -> None:
    write_profile(config_copy, "leaky", '[nats]\npassword = "hunter2"\n')
    with pytest.raises(s.SettingsError, match=r"nats\.password: secrets"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "leaky"})
    write_profile(config_copy, "leaky", '[llm.providers.anthropic]\napi_key = "sk-live"\n')
    with pytest.raises(s.SettingsError, match=r"llm\.providers\.anthropic\.api_key: secrets"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": "leaky"})
    with pytest.raises(s.SettingsError, match="secrets"):
        s.load(CONFIG_DIR, {**base_env, "WORKER__NATS__PASSWORD": "hunter2"})


@pytest.mark.parametrize("profile", ["gpu-12", "gpu-16"])
def test_small_card_profile_cannot_enable_trusted_lanes(
    base_env: dict[str, str], config_copy: Path, profile: str
) -> None:
    write_profile(config_copy, profile, '[lanes]\nenabled = ["audio", "encode"]\n')
    with pytest.raises(s.SettingsError, match="cannot enable \\['encode'\\]"):
        s.load(config_copy, {**base_env, "WORKER_PROFILE": profile})


@pytest.mark.parametrize(
    ("override", "message"),
    [
        ({"WORKER__WORKER__TRUST": "friend"}, "worker.trust"),
        ({"WORKER__RUNTIME__MODE": "single"}, "runtime.mode"),
        ({"WORKER__RUNTIME__DEVICE": "tpu"}, "runtime.device"),
        ({"WORKER__LANES__ENABLED": '["audio", "audio"]'}, "duplicates"),
        ({"WORKER__LANES__ENABLED": '["quality"]'}, "unknown lanes"),
        ({"WORKER__LANES__CAPACITY": "{ audio = 0 }"}, "lanes.capacity.audio"),
        ({"WORKER__SLOTS__ALIGN__FALLBACK": "whisper"}, "unknown slot"),
        ({"WORKER__SLOTS__MMS__REPLICAS": "0"}, "slots.mms.replicas"),
        ({"WORKER__SYNC__STRATEGY": "whisper"}, "sync.strategy"),
        ({"WORKER__SYNC__RESCUE_STRATEGY": "maybe"}, "sync.rescue_strategy"),
        ({"WORKER__SYNC__QUALITY__CONFIDENCE_MODEL": "calibrated"}, "non-zero confidence_weights"),
        ({"WORKER__LLM__PRIMARY": "gemini"}, "unknown provider"),
        ({"WORKER__LLM__FALLBACK": "anthropic"}, "must differ"),
        ({"WORKER__LLM__HEDGE_AFTER_SHARE": "1.5"}, "hedge_after_share"),
        ({"WORKER__NATS__PING": "{ interval_s = 0, max_outstanding = 2 }"}, "nats.ping"),
    ],
)
def test_validation_rejects_bad_values(
    base_env: dict[str, str], override: dict[str, str], message: str
) -> None:
    with pytest.raises(s.SettingsError, match=message):
        s.load(CONFIG_DIR, {**base_env, **override})


def test_calibrated_confidence_with_weights_is_accepted(base_env: dict[str, str]) -> None:
    env = {
        **base_env,
        "WORKER__SYNC__QUALITY__CONFIDENCE_MODEL": "calibrated",
        "WORKER__SYNC__QUALITY__CONFIDENCE_WEIGHTS": (
            "{ aligned_share = 1.2, inside_share = 0.4, collapsed = 0.3, anchor = 2.0,"
            " aligner = 0.5, separated = 0.2, order = 0.3, bias = -1.5 }"
        ),
    }
    settings = s.load(CONFIG_DIR, env)
    assert settings.sync.quality.confidence_model == "calibrated"
    assert settings.sync.quality.confidence_weights.bias == -1.5


def test_settings_are_frozen(settings: s.Settings) -> None:
    with pytest.raises(AttributeError):
        settings.worker.trust = "public"
    with pytest.raises(TypeError):
        settings.slots["muq"] = settings.slots["mulan"]


@pytest.mark.parametrize(
    ("raw", "expected"), [("7", 7), ("-3", -3), (" 12 ", 12), ("1.5", 1.5), ("1e3", 1000.0)]
)
def test_numeric_env_values_pick_int_or_float_by_their_text(raw: str, expected: object) -> None:
    value = s.coerce(raw, 0, "WORKER__RUNTIME__RECYCLE_AFTER_CALLS")
    assert value == expected
    assert type(value) is type(expected)


def test_settings_swallow_no_exception() -> None:
    tree = ast.parse(Path(s.__file__).read_text(encoding="utf-8"))
    swallowed = [
        handler.lineno
        for handler in ast.walk(tree)
        if isinstance(handler, ast.ExceptHandler)
        and all(isinstance(statement, ast.Pass) for statement in handler.body)
    ]
    assert swallowed == []
