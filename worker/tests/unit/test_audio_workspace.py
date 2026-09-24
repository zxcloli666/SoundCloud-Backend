from __future__ import annotations

from pathlib import Path

import pytest

from worker.domain.workspace import Workspace, safe_name
from worker.observability.counters import Counters


def test_purge_removes_leftovers_of_previous_run(work_dir: Path) -> None:
    (work_dir / "index_audio-1-abc").mkdir()
    (work_dir / "index_audio-1-abc" / "audio").write_bytes(b"x")
    (work_dir / "stray.tmp").write_bytes(b"y")

    removed = Workspace(work_dir, Counters()).purge()

    assert removed == 2
    assert list(work_dir.iterdir()) == []


def test_purge_creates_missing_root(tmp_path: Path) -> None:
    root = tmp_path / "work"

    assert Workspace(root, Counters()).purge() == 0
    assert root.is_dir()


def test_task_folder_is_removed_after_use(work_dir: Path) -> None:
    workspace = Workspace(work_dir, Counters())

    with workspace.task("index_audio-42") as folder:
        (folder / "audio").write_bytes(b"data")
        assert folder.parent == work_dir
        assert folder.name.startswith("index_audio-42-")

    assert list(work_dir.iterdir()) == []


def test_task_folder_is_removed_when_task_fails(work_dir: Path) -> None:
    workspace = Workspace(work_dir, Counters())

    with pytest.raises(RuntimeError), workspace.task("t") as folder:
        (folder / "audio").write_bytes(b"data")
        raise RuntimeError("boom")

    assert list(work_dir.iterdir()) == []


def test_two_tasks_with_one_name_get_separate_folders(work_dir: Path) -> None:
    workspace = Workspace(work_dir, Counters())

    with workspace.task("same") as first, workspace.task("same") as second:
        assert first != second


def test_cleanup_failure_is_counted(work_dir: Path) -> None:
    counters = Counters()
    workspace = Workspace(work_dir, counters)
    locked = work_dir / "locked"
    locked.mkdir()
    (locked / "file").write_bytes(b"x")
    locked.chmod(0o500)
    try:
        workspace.purge()
    finally:
        locked.chmod(0o700)

    assert counters.total("workspace_cleanup_failures_total") >= 1


@pytest.mark.parametrize(
    ("name", "safe"),
    [
        ("index_audio-98765", "index_audio-98765"),
        ("encode:mulan:ab/cd", "encode_mulan_ab_cd"),
        ("../../etc", "etc"),
        ("", "task"),
        ("x" * 200, "x" * 96),
    ],
)
def test_safe_name(name: str, safe: str) -> None:
    assert safe_name(name) == safe
