from __future__ import annotations

import ctypes
import gc

KIB_PER_MIB = 1024
RSS_FIELD = "VmRSS:"


def rss_mib(pid: int | str = "self") -> int:
    with open(f"/proc/{pid}/status", encoding="ascii") as handle:
        for line in handle:
            if line.startswith(RSS_FIELD):
                return int(line.split()[1]) // KIB_PER_MIB
    raise OSError(f"no {RSS_FIELD} in /proc/{pid}/status")


def trim(*, collect: bool = True) -> bool:
    if collect:
        gc.collect()
    try:
        ctypes.CDLL(None).malloc_trim(0)
    except (OSError, AttributeError):
        return False
    return True
