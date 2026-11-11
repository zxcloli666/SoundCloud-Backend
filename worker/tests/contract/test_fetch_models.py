from __future__ import annotations

import hashlib
from pathlib import Path
from types import SimpleNamespace

import orjson
import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR
from worker import fetch_models as fm
from worker import settings as s

PROFILES = sorted(path.stem for path in (CONFIG_DIR / "profiles").glob("*.toml"))
MULAN = "OpenMuQ/MuQ-MuLan-large"
FETCHED_AS_FILES = (fm.FASTTEXT_SLOT, fm.ROFORMER_SLOT, s.LOCAL_LLM_SLOT)


@pytest.fixture
def env(tmp_path: Path) -> dict[str, str]:
    return {
        **BASE_ENV,
        fm.FASTTEXT_HOME_ENV: str(tmp_path / "fasttext"),
        fm.ROFORMER_HOME_ENV: str(tmp_path / "roformer"),
    }


def plan_for(profile: str, env: dict[str, str]) -> list[fm.Item]:
    environ = {**env, s.PROFILE_ENV: profile}
    return fm.plan(s.load(CONFIG_DIR, environ), environ)


class FakeHub:
    def __init__(self, main_sha: str, files: list[str]) -> None:
        self.main_sha = main_sha
        self.files = files
        self.downloads: list[tuple[str, str]] = []

    def model_info(self, repo: str, revision: str) -> SimpleNamespace:
        return SimpleNamespace(id=repo, sha=self.main_sha)

    def list_repo_files(self, repo: str, revision: str) -> list[str]:
        return self.files

    def snapshot_download(self, repo: str, revision: str, allow_patterns: list[str]) -> str:
        self.downloads.append((repo, revision))
        resolved = self.main_sha if revision == fm.MAIN_BRANCH else revision
        return f"/models/hub/models--x/snapshots/{resolved}"


@pytest.mark.parametrize("profile", PROFILES)
def test_plan_fetches_exactly_the_models_the_profile_loads(
    profile: str, env: dict[str, str]
) -> None:
    environ = {**env, s.PROFILE_ENV: profile}
    settings = s.load(CONFIG_DIR, environ)
    items = fm.plan(settings, environ)
    hub = {item.artifact.repo for item in items if isinstance(item.artifact, fm.HubSnapshot)}
    expected = {
        settings.slots[slot].model
        for slot in s.slots_for_lanes(settings)
        if settings.slots[slot].model and slot not in FETCHED_AS_FILES
    }
    if MULAN in expected:
        expected |= {companion.repo for companion in fm.COMPANIONS[MULAN]}
    assert hub == expected
    for item in items:
        if isinstance(item.artifact, fm.HubSnapshot) and not item.artifact.loaded_by_name:
            assert item.artifact.revision == settings.slots[item.slot].revision
    assert len(items) == len(set(items))


def test_public_audio_node_fetches_no_sync_models(env: dict[str, str]) -> None:
    slots = {item.slot for item in plan_for("gpu-12-audio", env)}
    assert slots == {"muq", "mulan", "cpu-tools", "text"}


@pytest.mark.parametrize("profile", ["gpu-12", "gpu-16"])
def test_public_node_with_transcribe_fetches_every_public_lane_model(
    profile: str, env: dict[str, str]
) -> None:
    slots = {item.slot for item in plan_for(profile, env)}
    assert slots == {"muq", "mulan", "cpu-tools", "text", "sep", "asr", "align", "mms"}


def test_local_llm_weights_come_as_one_pinned_file_with_the_ai_lane(env: dict[str, str]) -> None:
    for profile in PROFILES:
        environ = {**env, s.PROFILE_ENV: profile}
        settings = s.load(CONFIG_DIR, environ)
        files = [item for item in fm.plan(settings, environ) if item.slot == s.LOCAL_LLM_SLOT]
        if "ai" not in settings.lanes.enabled:
            assert files == [], profile
            continue
        slot, local = settings.slots[s.LOCAL_LLM_SLOT], settings.llm.local
        assert [item.artifact for item in files] == [
            fm.HubFile(slot.model, slot.revision, local.file, local.sha256)
        ], profile


def test_hub_file_is_kept_only_at_its_pinned_commit_and_hash(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    commit = "a" * 40
    good = hashlib.sha256(b"gguf").hexdigest()
    stored = tmp_path / "store" / "12" / "chunks"
    blob = tmp_path / "blobs" / good
    link = tmp_path / "snapshots" / commit / "model.gguf"
    asked: list[tuple[str, str, str]] = []

    def cached(repo: str, filename: str, revision: str) -> str:
        asked.append((repo, filename, revision))
        for made in (stored, blob, link):
            made.parent.mkdir(parents=True, exist_ok=True)
            made.unlink(missing_ok=True)
        stored.write_bytes(b"gguf")
        blob.symlink_to(Path("..", "store", "12", "chunks"))
        link.symlink_to(Path("..", "..", "blobs", good))
        return str(link)

    monkeypatch.setattr(fm, "hf_hub_download", cached)
    fm.fetch_file(fm.HubFile("org/model-GGUF", commit, "model.gguf", good))
    assert asked == [("org/model-GGUF", "model.gguf", commit)]
    assert link.read_bytes() == b"gguf"
    with pytest.raises(fm.FetchError, match="sha256"):
        fm.fetch_file(fm.HubFile("org/model-GGUF", commit, "model.gguf", "0" * 64))
    assert not any(left.is_symlink() or left.exists() for left in (stored, blob, link))
    with pytest.raises(fm.FetchError, match="pinned commit"):
        fm.fetch_file(fm.HubFile("org/model-GGUF", "main", "model.gguf", good))


def test_verified_files_land_where_their_libraries_look(env: dict[str, str]) -> None:
    items = {item.slot: item.artifact for item in plan_for("gpu-24", env) if item.slot != "sep"}
    lid = items["cpu-tools"]
    assert isinstance(lid, fm.VerifiedDownload)
    assert lid.target == Path(env[fm.FASTTEXT_HOME_ENV]) / "lid.176.bin"
    roformer = [
        item.artifact
        for item in plan_for("gpu-24", env)
        if isinstance(item.artifact, fm.VerifiedDownload | fm.PackagedFile) and item.slot == "sep"
    ]
    slug_dir = Path(env[fm.ROFORMER_HOME_ENV]) / "melband-roformer-kim-vocals"
    assert [artifact.target for artifact in roformer] == [
        slug_dir / "MelBandRoformer.ckpt",
        slug_dir / "config_vocals_mel_band_roformer.yaml",
    ]


def test_missing_model_home_is_a_config_error(env: dict[str, str]) -> None:
    del env[fm.FASTTEXT_HOME_ENV]
    with pytest.raises(fm.FetchError, match=fm.FASTTEXT_HOME_ENV):
        plan_for("gpu-24", env)


def test_download_keeps_only_a_verified_file(tmp_path: Path) -> None:
    source = tmp_path / "lid.bin"
    source.write_bytes(b"weights")
    good = hashlib.sha256(b"weights").hexdigest()
    target = tmp_path / "models" / "lid.bin"
    with pytest.raises(fm.FetchError, match="sha256"):
        fm.fetch_download(fm.VerifiedDownload(source.as_uri(), "0" * 64, target))
    assert list(target.parent.iterdir()) == []
    fm.fetch_download(fm.VerifiedDownload(source.as_uri(), good, target))
    assert target.read_bytes() == b"weights"
    source.unlink()
    fm.fetch_download(fm.VerifiedDownload(source.as_uri(), good, target))


def test_packaged_roformer_config_matches_its_pin(tmp_path: Path) -> None:
    source = fm.ROFORMER_SOURCES["melband-roformer-kim-vocals"]
    target = tmp_path / "config.yaml"
    fm.fetch_packaged(
        fm.PackagedFile(
            fm.ROFORMER_PACKAGE, f"configs/{source.config}", source.config_sha256, target
        )
    )
    assert fm.has_sha256(target, source.config_sha256)
    with pytest.raises(fm.FetchError, match="sha256"):
        fm.fetch_packaged(
            fm.PackagedFile(
                fm.ROFORMER_PACKAGE, f"configs/{source.config}", "0" * 64, tmp_path / "x.yaml"
            )
        )


def test_snapshot_is_fetched_at_its_pinned_commit(monkeypatch: pytest.MonkeyPatch) -> None:
    hub = FakeHub(main_sha="f" * 40, files=["config.json"])
    monkeypatch.setattr(fm, "snapshot_download", hub.snapshot_download)
    fm.fetch_snapshot(fm.HubSnapshot("Qwen/Qwen3-Embedding-0.6B", "a" * 40), hub)
    assert hub.downloads == [("Qwen/Qwen3-Embedding-0.6B", "a" * 40)]


def test_companion_loaded_by_name_is_fetched_only_while_main_is_pinned(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    pinned = fm.HubSnapshot("xlm-roberta-base", "e" * 40, loaded_by_name=True)
    hub = FakeHub(main_sha="e" * 40, files=["config.json"])
    monkeypatch.setattr(fm, "snapshot_download", hub.snapshot_download)
    fm.fetch_snapshot(pinned, hub)
    assert hub.downloads == [("xlm-roberta-base", fm.MAIN_BRANCH)]
    moved = FakeHub(main_sha="d" * 40, files=["config.json"])
    monkeypatch.setattr(fm, "snapshot_download", moved.snapshot_download)
    with pytest.raises(fm.FetchError, match="main is at"):
        fm.fetch_snapshot(pinned, moved)
    assert moved.downloads == []


def test_chosen_files_prefer_safetensors_and_skip_other_formats() -> None:
    files = ["config.json", "model.safetensors", "pytorch_model.bin", "README.md", "onnx/m.onnx"]
    assert fm.chosen_files(files) == ["config.json", "model.safetensors"]
    assert fm.chosen_files(["config.json", "pytorch_model.bin"]) == [
        "config.json",
        "pytorch_model.bin",
    ]


def test_every_failure_is_reported_and_the_rest_still_fetched(
    capsys: pytest.CaptureFixture[str],
) -> None:
    items = [
        fm.Item("muq", fm.HubSnapshot("a/one", "1" * 40)),
        fm.Item("text", fm.HubSnapshot("a/two", "2" * 40)),
    ]

    def fetcher(artifact: fm.Artifact) -> None:
        if isinstance(artifact, fm.HubSnapshot) and artifact.repo == "a/one":
            raise OSError("connection reset")

    assert fm.fetch_all(items, fetcher) == 1
    events = [orjson.loads(line) for line in capsys.readouterr().out.splitlines()]
    assert [event["event"] for event in events] == ["model_fetch_failed", "model_fetched"]
    assert events[0]["slot"] == "muq" and "connection reset" in events[0]["error"]


def test_main_exit_codes(
    monkeypatch: pytest.MonkeyPatch, env: dict[str, str], capsys: pytest.CaptureFixture[str]
) -> None:
    config = ["--config-dir", str(CONFIG_DIR)]
    fetched: list[fm.Artifact] = []
    monkeypatch.setattr(fm, "fetch", fetched.append)
    assert fm.main(["--profile", "gpu-12-audio", *config], env) == fm.EXIT_OK
    assert fetched

    def broken(artifact: fm.Artifact) -> None:
        raise fm.FetchError("sha256 mismatch")

    monkeypatch.setattr(fm, "fetch", broken)
    assert fm.main(["--profile", "gpu-12-audio", *config], env) == fm.EXIT_FAILED
    assert fm.main(["--profile", "gpu-96", *config], env) == fm.EXIT_CONFIG
    monkeypatch.setattr(fm.constants, "HF_HUB_OFFLINE", True)
    assert fm.main(["--profile", "gpu-24", *config], env) == fm.EXIT_CONFIG
    last = orjson.loads(capsys.readouterr().out.splitlines()[-1])
    assert last["event"] == "fetch_models_config_error" and "HF_HUB_OFFLINE" in last["error"]
