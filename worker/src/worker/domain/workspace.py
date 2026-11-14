from __future__ import annotations

import fcntl
import logging
import os
import re
import shutil
import tempfile
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from functools import partial
from pathlib import Path

from worker.observability.counters import Counters

UNSAFE_NAME_CHARS = re.compile(r"[^A-Za-z0-9._-]+")
MAX_NAME_CHARS = 96
HOMES = "workers"

log = logging.getLogger(__name__)


class WorkRoot:
    def __init__(self, root: Path, owner: str, counters: Counters) -> None:
        self._homes = root / HOMES
        self._owner = owner
        self._counters = counters
        self._home: Path | None = None
        self._lock: int | None = None

    def claim(self) -> tuple[Path, int]:
        self._homes.mkdir(parents=True, exist_ok=True)
        self._home, self._lock = self._new_home()
        removed = 0
        for entry in self._homes.iterdir():
            held = try_lock(entry)
            if held is None:
                continue
            try:
                remove(entry, self._counters)
            finally:
                os.close(held)
            removed += 1
        return self._home, removed

    def release(self) -> None:
        if self._home is None or self._lock is None:
            return
        remove(self._home, self._counters)
        os.close(self._lock)
        self._home = None
        self._lock = None

    def _new_home(self) -> tuple[Path, int]:
        while True:
            home = Path(tempfile.mkdtemp(prefix=f"{safe_name(self._owner)}-", dir=self._homes))
            held = try_lock(home)
            if held is not None:
                return home, held


class Workspace:
    def __init__(self, root: Path, counters: Counters) -> None:
        self._root = root
        self._counters = counters

    @contextmanager
    def task(self, name: str) -> Iterator[Path]:
        self._root.mkdir(parents=True, exist_ok=True)
        path = Path(tempfile.mkdtemp(prefix=f"{safe_name(name)}-", dir=self._root))
        try:
            yield path
        finally:
            remove(path, self._counters)


def try_lock(path: Path) -> int | None:
    try:
        fd = os.open(path, os.O_RDONLY)
    except FileNotFoundError:
        return None
    if locked(fd, path):
        return fd
    os.close(fd)
    return None


def locked(fd: int, path: Path) -> bool:
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return False
    try:
        return os.path.samestat(os.fstat(fd), os.stat(path))
    except FileNotFoundError:
        return False


def remove(path: Path, counters: Counters) -> None:
    failed = partial(cleanup_failed, counters)
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path, onexc=failed)
        return
    try:
        path.unlink(missing_ok=True)
    except OSError as error:
        failed(Path.unlink, str(path), error)


def cleanup_failed(
    counters: Counters, function: Callable[..., object], path: str, error: BaseException
) -> None:
    counters.inc("workspace_cleanup_failures_total")
    log.error(
        "workspace cleanup failed",
        extra={"path": path, "operation": getattr(function, "__name__", str(function))},
        exc_info=error,
    )


def safe_name(name: str) -> str:
    cleaned = UNSAFE_NAME_CHARS.sub("_", name).strip("._")
    return cleaned[:MAX_NAME_CHARS] or "task"
