from __future__ import annotations

import logging
import re
import shutil
import tempfile
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path

from worker.observability.counters import Counters

UNSAFE_NAME_CHARS = re.compile(r"[^A-Za-z0-9._-]+")
MAX_NAME_CHARS = 96

log = logging.getLogger(__name__)


class Workspace:
    def __init__(self, root: Path, counters: Counters) -> None:
        self._root = root
        self._counters = counters

    def purge(self) -> int:
        self._root.mkdir(parents=True, exist_ok=True)
        removed = 0
        for entry in self._root.iterdir():
            self._remove(entry)
            removed += 1
        return removed

    @contextmanager
    def task(self, name: str) -> Iterator[Path]:
        self._root.mkdir(parents=True, exist_ok=True)
        path = Path(tempfile.mkdtemp(prefix=f"{safe_name(name)}-", dir=self._root))
        try:
            yield path
        finally:
            self._remove(path)

    def _remove(self, path: Path) -> None:
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path, onexc=self._cleanup_failed)
            return
        try:
            path.unlink(missing_ok=True)
        except OSError as error:
            self._cleanup_failed(Path.unlink, str(path), error)

    def _cleanup_failed(
        self, function: Callable[..., object], path: str, error: BaseException
    ) -> None:
        self._counters.inc("workspace_cleanup_failures_total")
        log.error(
            "workspace cleanup failed",
            extra={"path": path, "operation": getattr(function, "__name__", str(function))},
            exc_info=error,
        )


def safe_name(name: str) -> str:
    cleaned = UNSAFE_NAME_CHARS.sub("_", name).strip("._")
    return cleaned[:MAX_NAME_CHARS] or "task"
