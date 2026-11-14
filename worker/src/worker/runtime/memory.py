from __future__ import annotations

import ctypes
import gc
from dataclasses import dataclass
from pathlib import Path

KIB_PER_MIB = 1024
RSS_FIELD = "VmRSS:"
ANON_FIELD = "RssAnon:"
SHMEM_FIELD = "RssShmem:"
USAGE_FIELDS = frozenset({RSS_FIELD, ANON_FIELD, SHMEM_FIELD})
PROC = Path("/proc")
EXITED_STATES = frozenset({"Z", "X"})


@dataclass(frozen=True)
class Usage:
    resident_mib: int
    charged_mib: int


NO_USAGE = Usage(0, 0)


def rss_mib(pid: int | str = "self") -> int:
    return rss_kib(pid) // KIB_PER_MIB


def tree_usage(pid: int) -> Usage:
    own = status_kib(pid)
    if RSS_FIELD not in own:
        raise OSError(f"no {RSS_FIELD} in /proc/{pid}/status")
    tree = [own, *(child_status_kib(child) for child in descendants(pid))]
    resident = sum(fields.get(RSS_FIELD, 0) for fields in tree)
    charged = sum(fields.get(ANON_FIELD, 0) + fields.get(SHMEM_FIELD, 0) for fields in tree)
    return Usage(resident // KIB_PER_MIB, charged // KIB_PER_MIB)


def rss_kib(pid: int | str) -> int:
    resident = status_kib(pid).get(RSS_FIELD)
    if resident is None:
        raise OSError(f"no {RSS_FIELD} in /proc/{pid}/status")
    return resident


def status_kib(pid: int | str) -> dict[str, int]:
    found: dict[str, int] = {}
    with open(f"/proc/{pid}/status", encoding="ascii", errors="replace") as handle:
        for line in handle:
            field, _, value = line.partition("\t")
            if field in USAGE_FIELDS:
                found[field] = int(value.split()[0])
    return found


def child_status_kib(pid: int) -> dict[str, int]:
    try:
        return status_kib(pid)
    except (FileNotFoundError, ProcessLookupError):
        return {}


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
