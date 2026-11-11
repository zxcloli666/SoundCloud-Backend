from __future__ import annotations

import json
import math
import platform
import sys
import time
from typing import TYPE_CHECKING

from worker import fetch_models
from worker.runtime import cpu_flags
from worker.runtime.protocol import CALL_GUARD, CallExpired, SlotSpec
from worker.settings import LOCAL_LLM_DEVICE, LOCAL_LLM_SLOT

if TYPE_CHECKING:
    from worker.models.local_llm.slot import LocalLlmSlot

REPO = "unsloth/SmolLM2-135M-Instruct-GGUF"
REVISION = "9e6855bc4be717fca1ef21360a1db4b29d5c559a"
FILE = "SmolLM2-135M-Instruct-Q4_K_M.gguf"
SHA256 = "ed5fa30c487b282ec156c29062f1222e5c20875a944ac98289dbd242e947f747"
FETCH_ATTEMPTS = 4
FETCH_PAUSE_S = 15.0
THREADS = 2
BUILD_FLAGS = {
    "SSE3": "pni",
    "SSSE3": "ssse3",
    "AVX": "avx",
    "AVX2": "avx2",
    "F16C": "f16c",
    "FMA": "fma",
    "BMI2": "bmi2",
    "AVX_VNNI": "avx_vnni",
    "AVX512": "avx512f",
    "AVX512_VBMI": "avx512vbmi",
    "AVX512_VNNI": "avx512_vnni",
    "AVX512_BF16": "avx512_bf16",
    "AMX_INT8": "amx_int8",
    "NEON": "asimd",
    "ARM_FMA": "asimd",
    "FP16_VA": "asimdhp",
    "DOTPROD": "asimddp",
    "MATMUL_INT8": "i8mm",
    "SVE": "sve",
    "SME": "sme",
}
UNMAPPED = frozenset({"LLAMAFILE", "OPENMP", "REPACK", "ACCELERATE", "KLEIDIAI"})
SCHEMA = {
    "type": "object",
    "properties": {"ok": {"const": True}},
    "required": ["ok"],
    "additionalProperties": False,
}
SPEC = SlotSpec(
    name=LOCAL_LLM_SLOT,
    loader="worker.models.local_llm.slot:LocalLlmSlot",
    model=REPO,
    revision=REVISION,
    device=LOCAL_LLM_DEVICE,
    max_batch=1,
    max_wait_ms=0,
    options={"file": FILE, "context_tokens": 2048, "chunk_tokens": 64, "state_cache_mib": 64},
)


def main() -> int:
    missing = cpu_flags.missing_on_this_host()
    if missing:
        return failed("this host lacks the cpu flags of the llama.cpp build", missing=missing)
    import llama_cpp

    built = built_flags(llama_cpp.llama_print_system_info().decode())
    unchecked = sorted(built - cpu_flags.LLAMA_CPU_FLAGS[platform.machine()])
    if unchecked:
        return failed("the wheel needs flags the start check does not ask for", flags=unchecked)
    fetch()
    from worker.models.local_llm.slot import LocalLlmSlot

    CALL_GUARD.threads = THREADS
    slot = LocalLlmSlot()
    slot.load(SPEC)
    slot.warmup()
    answer = ask(slot)
    if json.loads(str(answer["text"])) != {"ok": True} or answer["finished"] is not True:
        return failed("the grammar did not hold the answer", answer=answer)
    CALL_GUARD.expires_at = time.monotonic() - 1.0
    try:
        ask(slot)
    except CallExpired:
        pass
    else:
        return failed("a call past its deadline was not stopped")
    finally:
        CALL_GUARD.expires_at = math.inf
    after = ask(slot)
    slot.unload()
    print(json.dumps({"smoke": "ok", "flags": sorted(built), "answer": after["text"]}))
    return 0


def built_flags(info: str) -> set[str]:
    names = set()
    for part in info.split("|"):
        name, _, value = part.rpartition(":")[2].partition("=")
        name = name.strip()
        if value.strip() != "1" or name in UNMAPPED:
            continue
        if name not in BUILD_FLAGS:
            raise SystemExit(f"unknown llama.cpp build flag {name}: map it to a cpuinfo flag")
        names.add(BUILD_FLAGS[name])
    return names


def fetch() -> None:
    artifact = fetch_models.HubFile(REPO, REVISION, FILE, SHA256)
    for attempt in range(1, FETCH_ATTEMPTS + 1):
        try:
            fetch_models.fetch(artifact)
            return
        except fetch_models.FETCH_ERRORS:
            if attempt == FETCH_ATTEMPTS:
                raise
            time.sleep(FETCH_PAUSE_S)


def ask(slot: LocalLlmSlot) -> dict[str, object]:
    messages = [{"role": "user", "content": "Are you ok?"}]
    return dict(slot.complete("Answer with one JSON object.", (), messages, SCHEMA, 32))


def failed(reason: str, **fields: object) -> int:
    print(json.dumps({"smoke": "failed", "reason": reason, **fields}, default=str))
    return 1


if __name__ == "__main__":
    sys.exit(main())
