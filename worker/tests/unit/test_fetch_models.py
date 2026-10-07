from __future__ import annotations

import hashlib
from pathlib import Path
from types import SimpleNamespace

import pytest
from huggingface_hub import try_to_load_from_cache

from worker import fetch_models as fm

LEGACY = "xlm-roberta-base"
CANONICAL = "FacebookAI/xlm-roberta-base"
REVISION = "e" * 40
FILES = {"config.json": b"{}", "tokenizer.json": b'{"tok": 1}'}


class RenamedHub:
    def __init__(self, cache: Path) -> None:
        self.cache = cache
        self.listed: list[str] = []
        self.downloads: list[tuple[str, str]] = []

    def model_info(self, repo: str, revision: str) -> SimpleNamespace:
        return SimpleNamespace(id=CANONICAL, sha=REVISION)

    def list_repo_files(self, repo: str, revision: str) -> list[str]:
        self.listed.append(repo)
        return list(FILES)

    def snapshot_download(self, repo: str, revision: str, allow_patterns: list[str]) -> str:
        self.downloads.append((repo, revision))
        root = self.cache / ("models--" + repo.replace("/", "--"))
        snapshot = root / "snapshots" / REVISION
        snapshot.mkdir(parents=True)
        (root / "blobs").mkdir()
        for name in allow_patterns:
            blob = root / "blobs" / hashlib.sha256(FILES[name]).hexdigest()
            blob.write_bytes(FILES[name])
            (snapshot / name).symlink_to(Path("..", "..", "blobs", blob.name))
        (root / "refs").mkdir()
        (root / "refs" / revision).write_text(REVISION)
        return str(snapshot)


def test_renamed_companion_downloads_by_canonical_id_and_resolves_by_legacy_name(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    hub = RenamedHub(tmp_path)
    monkeypatch.setattr(fm, "snapshot_download", hub.snapshot_download)
    fm.fetch_snapshot(fm.HubSnapshot(LEGACY, REVISION, loaded_by_name=True), hub)
    assert hub.listed == [CANONICAL]
    assert hub.downloads == [(CANONICAL, fm.MAIN_BRANCH)]
    legacy = tmp_path / "models--xlm-roberta-base"
    assert (legacy / "refs" / fm.MAIN_BRANCH).read_text() == REVISION
    for name, content in FILES.items():
        cached = try_to_load_from_cache(LEGACY, name, cache_dir=tmp_path)
        assert cached == str(legacy / "snapshots" / REVISION / name)
        assert Path(cached).read_bytes() == content
        assert not (legacy / "snapshots" / REVISION / name).readlink().is_absolute()


def test_relinking_keeps_files_already_cached_under_the_legacy_name(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    kept = tmp_path / "models--xlm-roberta-base" / "snapshots" / REVISION / "config.json"
    kept.parent.mkdir(parents=True)
    kept.write_bytes(b'{"old": true}')
    hub = RenamedHub(tmp_path)
    monkeypatch.setattr(fm, "snapshot_download", hub.snapshot_download)
    fm.fetch_snapshot(fm.HubSnapshot(LEGACY, REVISION, loaded_by_name=True), hub)
    assert kept.read_bytes() == b'{"old": true}'
    assert try_to_load_from_cache(LEGACY, "tokenizer.json", cache_dir=tmp_path) is not None
