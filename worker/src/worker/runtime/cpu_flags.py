from __future__ import annotations

import platform
from collections.abc import Mapping
from pathlib import Path

CPUINFO = Path("/proc/cpuinfo")
FLAG_KEYS = frozenset({"flags", "Features"})
LLAMA_CPU_FLAGS: Mapping[str, frozenset[str]] = {
    "x86_64": frozenset({"pni", "ssse3", "avx", "avx2", "f16c", "fma", "bmi2"}),
    "aarch64": frozenset({"fp", "asimd"}),
}


def missing_llama_flags(machine: str, cpuinfo: str) -> list[str]:
    needed = LLAMA_CPU_FLAGS.get(machine)
    if needed is None:
        return [f"llama.cpp build for {machine}"]
    cores = [
        set(value.split())
        for key, _, value in (line.partition(":") for line in cpuinfo.splitlines())
        if key.strip() in FLAG_KEYS
    ]
    present = set.intersection(*cores) if cores else set()
    return sorted(needed - present)


def missing_on_this_host() -> list[str]:
    try:
        cpuinfo = CPUINFO.read_text(encoding="ascii", errors="replace")
    except OSError:
        return [f"readable {CPUINFO}"]
    return missing_llama_flags(platform.machine(), cpuinfo)
