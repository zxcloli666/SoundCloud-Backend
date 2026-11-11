from __future__ import annotations

from pathlib import Path

import pytest

from worker.runtime import cpu_flags
from worker.runtime.cpu_flags import missing_llama_flags

X86_FULL = "fpu sse2 pni ssse3 fma sse4_2 avx f16c bmi1 avx2 bmi2"
X86_OLD = "fpu sse2 pni ssse3 sse4_2 avx"
ARM = "fp asimd evtstrm aes pmull sha1 sha2 crc32"


def cpuinfo(key: str, *cores: str) -> str:
    return "\n\n".join(f"processor\t: {n}\n{key}\t\t: {flags}" for n, flags in enumerate(cores))


@pytest.mark.parametrize(
    ("machine", "text", "missing"),
    [
        ("x86_64", cpuinfo("flags", X86_FULL, X86_FULL), []),
        ("x86_64", cpuinfo("flags", X86_OLD), ["avx2", "bmi2", "f16c", "fma"]),
        ("x86_64", cpuinfo("flags", X86_FULL, X86_OLD), ["avx2", "bmi2", "f16c", "fma"]),
        ("aarch64", cpuinfo("Features", ARM), []),
        ("aarch64", "", ["asimd", "fp"]),
        ("riscv64", cpuinfo("isa", "rv64imafdc"), ["llama.cpp build for riscv64"]),
    ],
)
def test_the_flags_the_llama_build_needs_are_read_from_every_core(
    machine: str, text: str, missing: list[str]
) -> None:
    assert missing_llama_flags(machine, text) == missing


def test_an_unreadable_cpuinfo_counts_as_unsupported(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    absent = tmp_path / "cpuinfo"
    monkeypatch.setattr(cpu_flags, "CPUINFO", absent)
    assert cpu_flags.missing_on_this_host() == [f"readable {absent}"]
