from __future__ import annotations

import os

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
