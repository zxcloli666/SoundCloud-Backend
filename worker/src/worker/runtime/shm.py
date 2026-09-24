from __future__ import annotations

import contextlib
import mmap
import os
import re
from collections.abc import Mapping
from pathlib import Path

import numpy as np

from worker.runtime.protocol import ArrayRef, Arrays

SHM_DIR = Path("/dev/shm")
PREFIX = "wk-"
KEY_CHARS = re.compile(r"[^A-Za-z0-9_]")


class SharedBlocks:
    def __init__(self, main_pid: int, engine_pid: int, call_id: int, tag: str) -> None:
        self._stem = f"{PREFIX}{main_pid}-{engine_pid}-{call_id}-{tag}-"
        self._names: list[str] = []

    def share(self, arrays: Arrays) -> dict[str, ArrayRef]:
        refs: dict[str, ArrayRef] = {}
        try:
            for key, array in arrays.items():
                name = self._stem + KEY_CHARS.sub("_", key)
                refs[key] = write(name, array)
                if refs[key].shm_name:
                    self._names.append(name)
        except BaseException:
            self.release()
            raise
        return refs

    def release(self) -> None:
        for name in self._names:
            unlink(name)
        self._names.clear()


def write(name: str, array: np.ndarray) -> ArrayRef:
    if array.dtype.hasobject:
        raise ValueError(f"{name}: object arrays cannot be shared")
    contiguous = np.ascontiguousarray(array)
    ref = ArrayRef(name, tuple(contiguous.shape), contiguous.dtype.str)
    if contiguous.nbytes == 0:
        return ArrayRef("", ref.shape, ref.dtype)
    fd = os.open(SHM_DIR / name, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    try:
        os.ftruncate(fd, contiguous.nbytes)
        os.posix_fallocate(fd, 0, contiguous.nbytes)
        mapped = mmap.mmap(fd, contiguous.nbytes)
        try:
            view = np.ndarray(contiguous.shape, contiguous.dtype, buffer=mapped)
            view[...] = contiguous
            del view
        finally:
            mapped.close()
    except BaseException:
        unlink(name)
        raise
    finally:
        os.close(fd)
    return ref


def read(ref: ArrayRef) -> np.ndarray:
    dtype = np.dtype(ref.dtype)
    if not ref.shm_name:
        return np.empty(ref.shape, dtype)
    fd = os.open(SHM_DIR / ref.shm_name, os.O_RDONLY)
    try:
        mapped = mmap.mmap(fd, 0, access=mmap.ACCESS_READ)
        try:
            view = np.ndarray(ref.shape, dtype, buffer=mapped)
            copy = view.copy()
            del view
        finally:
            mapped.close()
    finally:
        os.close(fd)
    return copy


def read_all(refs: Mapping[str, ArrayRef]) -> dict[str, np.ndarray]:
    return {key: read(ref) for key, ref in refs.items()}


def unlink(name: str) -> None:
    with contextlib.suppress(FileNotFoundError):
        os.unlink(SHM_DIR / name)


def take_all(refs: Mapping[str, ArrayRef]) -> dict[str, np.ndarray]:
    try:
        return read_all(refs)
    finally:
        for ref in refs.values():
            if ref.shm_name:
                unlink(ref.shm_name)


def engine_prefix(main_pid: int, engine_pid: int) -> str:
    return f"{PREFIX}{main_pid}-{engine_pid}-"


def sweep(prefix: str) -> int:
    removed = 0
    for entry in SHM_DIR.iterdir():
        if entry.name.startswith(prefix):
            unlink(entry.name)
            removed += 1
    return removed


def sweep_dead_owners() -> int:
    removed = 0
    for entry in SHM_DIR.iterdir():
        if not entry.name.startswith(PREFIX):
            continue
        owner = entry.name[len(PREFIX) :].split("-", 1)[0]
        if owner.isdigit() and not process_alive(int(owner)):
            unlink(entry.name)
            removed += 1
    return removed


def process_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True
