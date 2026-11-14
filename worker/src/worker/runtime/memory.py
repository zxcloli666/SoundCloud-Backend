from __future__ import annotations

import ctypes
import gc
from pathlib import Path

KIB_PER_MIB = 1024
RSS_FIELD = "VmRSS:"
PROC = Path("/proc")
EXITED_STATES = frozenset({"Z", "X"})


def rss_mib(pid: int | str = "self") -> int:
    return rss_kib(pid) // KIB_PER_MIB


def tree_rss_mib(pid: int) -> int:
    children = sum(child_kib(child) for child in descendants(pid))
    return (rss_kib(pid) + children) // KIB_PER_MIB


def rss_kib(pid: int | str) -> int:
    resident = resident_kib(pid)
    if resident is None:
        raise OSError(f"no {RSS_FIELD} in /proc/{pid}/status")
    return resident


def resident_kib(pid: int | str) -> int | None:
    with open(f"/proc/{pid}/status", encoding="ascii") as handle:
        for line in handle:
            if line.startswith(RSS_FIELD):
                return int(line.split()[1])
    return None


def child_kib(pid: int) -> int:
    try:
        return resident_kib(pid) or 0
    except (FileNotFoundError, ProcessLookupError):
        return 0


def descendants(pid: int) -> list[int]:
    parents = live_parents()
    found: list[int] = []
    waiting = [pid]
    while waiting:
        parent = waiting.pop()
        children = [child for child, of in parents.items() if of == parent]
        found += children
        waiting += children
    return found


def live_parents() -> dict[int, int]:
    parents: dict[int, int] = {}
    for entry in PROC.iterdir():
        if not entry.name.isdigit():
            continue
        parent = live_parent(entry)
        if parent is not None:
            parents[int(entry.name)] = parent
    return parents


def live_parent(entry: Path) -> int | None:
    try:
        stat = (entry / "stat").read_text(encoding="ascii", errors="replace")
    except (FileNotFoundError, ProcessLookupError):
        return None
    state, parent = stat.rpartition(")")[2].split()[:2]
    if state in EXITED_STATES:
        return None
    return int(parent)


def trim(*, collect: bool = True) -> bool:
    if collect:
        gc.collect()
    try:
        ctypes.CDLL(None).malloc_trim(0)
    except (OSError, AttributeError):
        return False
    return True
