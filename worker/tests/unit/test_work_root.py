from __future__ import annotations

import subprocess
import sys
import threading
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from worker.domain.workspace import HOMES, WorkRoot, try_lock
from worker.observability.counters import Counters

WORKER = (
    "import sys\n"
    "from pathlib import Path\n"
    "from worker.domain.workspace import WorkRoot\n"
    "from worker.observability.counters import Counters\n"
    "home, _ = WorkRoot(Path(sys.argv[1]), sys.argv[2], Counters()).claim()\n"
    "(home / 'audio').write_bytes(b'x')\n"
    "print(home, flush=True)\n"
    "sys.stdin.read()\n"
)


def test_two_workers_on_one_root_get_their_own_homes(work_dir: Path) -> None:
    first = WorkRoot(work_dir, "gpu-main", Counters())
    second = WorkRoot(work_dir, "gpu-tts", Counters())

    first_home, _ = first.claim()
    second_home, removed = second.claim()

    assert first_home != second_home
    assert first_home.is_dir() and second_home.is_dir()
    assert first_home.name.startswith("gpu-main-")
    assert second_home.name.startswith("gpu-tts-")
    assert removed == 0
    first.release()
    second.release()


def test_a_live_worker_in_another_process_keeps_its_files_when_another_starts(
    work_dir: Path,
) -> None:
    with live_worker(work_dir, "gpu-main") as (home, _):
        _, removed = WorkRoot(work_dir, "gpu-tts", Counters()).claim()

        assert removed == 0
        assert (home / "audio").read_bytes() == b"x"


def test_a_dead_workers_home_is_removed_by_the_next_start(work_dir: Path) -> None:
    with live_worker(work_dir, "gpu-old") as (home, worker):
        worker.kill()
        worker.wait()

        _, removed = WorkRoot(work_dir, "gpu-tts", Counters()).claim()

    assert removed == 1
    assert not home.exists()


def test_own_leftovers_of_an_earlier_run_are_removed_at_start(work_dir: Path) -> None:
    with live_worker(work_dir, "gpu-main") as (earlier, worker):
        worker.kill()
        worker.wait()

        home, removed = WorkRoot(work_dir, "gpu-main", Counters()).claim()

    assert removed == 1
    assert not earlier.exists()
    assert list((work_dir / HOMES).iterdir()) == [home]


def test_a_home_left_before_its_lock_was_taken_is_removed(work_dir: Path) -> None:
    unlocked = work_dir / HOMES / "gpu-main-died"
    unlocked.mkdir(parents=True)

    home, removed = WorkRoot(work_dir, "gpu-tts", Counters()).claim()

    assert removed == 1
    assert list((work_dir / HOMES).iterdir()) == [home]


def test_a_home_removed_before_it_was_locked_cannot_be_taken(work_dir: Path) -> None:
    taken = work_dir / HOMES / "gpu-main-taken"
    taken.mkdir(parents=True)
    taken.rmdir()

    assert try_lock(taken) is None


def test_workers_starting_at_the_same_moment_never_share_a_home(work_dir: Path) -> None:
    roots = [WorkRoot(work_dir, f"gpu-{number}", Counters()) for number in range(8)]
    start = threading.Barrier(len(roots))
    homes: list[Path] = []

    def claim(root: WorkRoot) -> None:
        start.wait()
        homes.append(root.claim()[0])

    threads = [threading.Thread(target=claim, args=(root,)) for root in roots]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert len(set(homes)) == len(roots)
    assert set((work_dir / HOMES).iterdir()) == set(homes)
    assert all(try_lock(home) is None for home in homes)
    for root in roots:
        root.release()


def test_release_removes_the_home_and_frees_nothing_else(work_dir: Path) -> None:
    first = WorkRoot(work_dir, "gpu-main", Counters())
    second = WorkRoot(work_dir, "gpu-tts", Counters())
    first_home, _ = first.claim()
    second_home, _ = second.claim()

    first.release()

    assert not first_home.exists()
    assert second_home.is_dir() and try_lock(second_home) is None
    second.release()


def test_entries_of_the_old_layout_in_the_root_are_left_alone(work_dir: Path) -> None:
    (work_dir / "index_audio-1-abc").mkdir()
    (work_dir / "stray.tmp").write_bytes(b"y")

    WorkRoot(work_dir, "gpu-main", Counters()).claim()

    assert (work_dir / "index_audio-1-abc").is_dir()
    assert (work_dir / "stray.tmp").read_bytes() == b"y"


def test_a_cleanup_failure_is_counted(work_dir: Path) -> None:
    counters = Counters()
    stuck = work_dir / HOMES / "gpu-old-stuck"
    (stuck / "audio").mkdir(parents=True)
    (stuck / "audio" / "file").write_bytes(b"x")
    (stuck / "audio").chmod(0o500)
    try:
        WorkRoot(work_dir, "gpu-main", counters).claim()
    finally:
        if (stuck / "audio").exists():
            (stuck / "audio").chmod(0o700)

    assert counters.total("workspace_cleanup_failures_total") >= 1


@contextmanager
def live_worker(root: Path, owner: str) -> Iterator[tuple[Path, subprocess.Popen[str]]]:
    with subprocess.Popen(
        [sys.executable, "-c", WORKER, str(root), owner],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    ) as worker:
        try:
            assert worker.stdout is not None
            yield Path(worker.stdout.readline().strip()), worker
        finally:
            worker.kill()
