from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

from worker.runtime import memory


def test_rss_of_this_process_is_read_from_proc() -> None:
    assert memory.rss_mib() > 0
    assert memory.rss_mib(os.getpid()) > 0


def test_rss_of_a_missing_process_is_an_error() -> None:
    with pytest.raises(OSError):
        memory.rss_mib(2**22 + 1)


def test_trim_returns_freed_heap_to_the_system() -> None:
    assert memory.trim()


BALLAST_MIB = 64
HOLDER = (
    "import sys, time\n"
    "ballast = bytes(range(256)) * (int(sys.argv[1]) << 12)\n"
    "print('ready', flush=True)\n"
    "time.sleep(60)\n"
)
PARENT = (
    "import subprocess, sys\n"
    "subprocess.run([sys.executable, '-c', sys.argv[1], sys.argv[2]], check=False)\n"
)
MAPPER = (
    "import mmap, sys, time\n"
    "with open(sys.argv[1], 'rb') as model:\n"
    "    mapped = mmap.mmap(model.fileno(), 0, prot=mmap.PROT_READ)\n"
    "touched = sum(mapped[offset] for offset in range(0, len(mapped), mmap.PAGESIZE))\n"
    "print('ready', flush=True)\n"
    "time.sleep(60)\n"
)


def started(*args: str) -> subprocess.Popen[str]:
    process = subprocess.Popen(
        [sys.executable, "-c", *args], stdout=subprocess.PIPE, text=True, start_new_session=True
    )
    assert process.stdout is not None and process.stdout.readline().strip() == "ready"
    return process


def killed(process: subprocess.Popen[str]) -> None:
    os.killpg(process.pid, signal.SIGKILL)
    process.wait()


@pytest.fixture
def family() -> Iterator[subprocess.Popen[str]]:
    parent = started(PARENT, HOLDER, str(BALLAST_MIB))
    yield parent
    killed(parent)


@pytest.fixture
def mapper(tmp_path: Path) -> Iterator[subprocess.Popen[str]]:
    model = tmp_path / "model.bin"
    model.write_bytes(bytes(range(256)) * (BALLAST_MIB << 12))
    process = started(MAPPER, str(model))
    yield process
    killed(process)


def test_the_memory_of_a_process_counts_its_children_and_grandchildren(
    family: subprocess.Popen[str],
) -> None:
    [holder] = memory.descendants(family.pid)
    usage = memory.tree_usage(family.pid)
    ours = memory.tree_usage(os.getpid())

    assert {family.pid, holder} <= set(memory.descendants(os.getpid()))
    assert usage.resident_mib - memory.rss_mib(family.pid) >= BALLAST_MIB
    assert usage.charged_mib >= BALLAST_MIB
    assert ours.resident_mib - memory.rss_mib(os.getpid()) >= BALLAST_MIB
    assert ours.charged_mib >= usage.charged_mib


def test_mapped_model_files_count_as_resident_but_not_as_charged(
    mapper: subprocess.Popen[str],
) -> None:
    usage = memory.tree_usage(mapper.pid)

    assert usage.resident_mib - usage.charged_mib >= BALLAST_MIB
    assert usage.charged_mib < BALLAST_MIB


def test_the_memory_of_a_missing_process_is_an_error() -> None:
    with pytest.raises(OSError):
        memory.tree_usage(2**22 + 1)


def test_a_child_that_already_exited_is_not_counted() -> None:
    child = subprocess.Popen([sys.executable, "-c", "pass"])
    try:
        assert wait_for_state(child.pid, "Z")

        assert child.pid not in memory.descendants(os.getpid())
        assert memory.tree_usage(os.getpid()).resident_mib >= memory.rss_mib(os.getpid())
    finally:
        child.wait()


def wait_for_state(pid: int, state: str) -> bool:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
        if stat.rpartition(")")[2].split()[0] == state:
            return True
        time.sleep(0.02)
    return False
