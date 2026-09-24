from __future__ import annotations

import atexit
import os
import signal
import time
from collections.abc import Mapping

import numpy as np

from worker.runtime.protocol import Arrays, BadInput, SlotSpec


class Fake:
    def __init__(self) -> None:
        self.spec: SlotSpec | None = None
        self.loaded = False

    def load(self, spec: SlotSpec) -> None:
        self.spec = spec
        if spec.options.get("fail_load"):
            raise RuntimeError("weights missing")
        marker = spec.options.get("fail_load_once")
        if isinstance(marker, str) and os.path.exists(marker):
            os.unlink(marker)
            raise RuntimeError("weights missing once")
        pause(spec.options.get("load_delay_s"))
        exit_delay = spec.options.get("exit_delay_s")
        if exit_delay:
            atexit.register(pause, exit_delay)
        self.loaded = True

    def warmup(self) -> None:
        return None

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        rows = row_count(arrays, args)
        if method == "hang" or any(flags(args.get("hang"), rows)):
            while True:
                time.sleep(1)
        if method == "crash":
            os._exit(int(str(args.get("code", 7))))
        if method == "segv":
            os.kill(os.getpid(), signal.SIGSEGV)
        if method == "orphan":
            leave_orphan(str(args["pid_file"]))
        if method == "bad":
            raise BadInput("bad row")
        if method == "boom":
            raise ValueError("model exploded")
        if method == "oom" and rows > int(str(args.get("fits", 1))):
            raise RuntimeError("CUDA out of memory. Tried to allocate 1.00 GiB")
        if method == "shape":
            return {"out": np.zeros((rows + 1, 2), dtype=np.float32)}, {}
        seconds = args.get("seconds", 0)
        if isinstance(seconds, int | float) and seconds > 0:
            time.sleep(seconds)
        outputs, result = echo(arrays, args, rows)
        result["device"] = self.spec.device if self.spec is not None else ""
        return outputs, result

    def unload(self) -> None:
        options = self.spec.options if self.spec is not None else {}
        if options.get("unload_crash"):
            os._exit(9)
        pause(options.get("unload_delay_s"))
        self.loaded = False


def pause(seconds: object) -> None:
    if isinstance(seconds, int | float) and seconds > 0:
        time.sleep(seconds)


def leave_orphan(pid_file: str) -> None:
    child = os.fork()
    if child == 0:
        time.sleep(60)
        os._exit(0)
    with open(pid_file, "w", encoding="ascii") as handle:
        handle.write(str(child))
    os._exit(7)


def echo(arrays: Arrays, args: Mapping[str, object], rows: int) -> tuple[Arrays, dict[str, object]]:
    scale = args.get("scale", 2)
    factor = scale if isinstance(scale, int | float) else 2
    outputs = {key: value * factor for key, value in arrays.items()}
    tags = args.get("tags")
    result: dict[str, object] = {"rows": rows, "pid": os.getpid(), "scale": factor}
    if isinstance(tags, list):
        result["tags"] = [f"{tag}!" for tag in tags]
    return outputs, result


def row_count(arrays: Arrays, args: Mapping[str, object]) -> int:
    for value in arrays.values():
        return int(value.shape[0])
    for value in args.values():
        if isinstance(value, list):
            return len(value)
    return 1


def flags(value: object, rows: int) -> list[bool]:
    if isinstance(value, list):
        return [bool(item) for item in value]
    return [bool(value)] * rows
