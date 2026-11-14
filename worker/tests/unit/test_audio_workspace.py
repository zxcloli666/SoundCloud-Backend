from __future__ import annotations

from pathlib import Path

import pytest

from worker.domain.workspace import Workspace, safe_name
from worker.observability.counters import Counters


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
