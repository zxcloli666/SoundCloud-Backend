from __future__ import annotations

import logging
import os
import sys
import time
import traceback
from collections.abc import Callable
from typing import TextIO

import orjson

LEVELS = {"debug": 10, "info": 20, "warning": 30, "error": 40}
ERROR_TEXT_LIMIT = 512
TRACEBACK_LIMIT = 4096
RECORD_ATTRIBUTES = frozenset(
    logging.LogRecord("", logging.INFO, "", 0, "", None, None).__dict__
) | {"message", "asctime"}


class JsonLog:
    def __init__(
        self,
        stream: TextIO | None = None,
        *,
        level: str = "info",
        wall: Callable[[], float] = time.time,
        **fields: object,
    ) -> None:
        self._stream = stream
        self._threshold = LEVELS[level]
        self._wall = wall
        self._fields = dict(fields)

    def bind(self, **fields: object) -> JsonLog:
        return JsonLog(
            self._stream,
            level=level_name(self._threshold),
            wall=self._wall,
            **{**self._fields, **fields},
        )

    def debug(self, event: str, **fields: object) -> None:
        self.write("debug", event, fields)

    def info(self, event: str, **fields: object) -> None:
        self.write("info", event, fields)

    def warning(self, event: str, **fields: object) -> None:
        self.write("warning", event, fields)

    def error(self, event: str, **fields: object) -> None:
        self.write("error", event, fields)

    def exception(self, event: str, error: BaseException, **fields: object) -> None:
        self.write(
            "error",
            event,
            {
                **fields,
                "error": error_text(error),
                "traceback": traceback_text(error),
            },
        )

    def write(self, level: str, event: str, fields: dict[str, object]) -> None:
        if LEVELS[level] < self._threshold:
            return
        record = {
            "ts": round(self._wall(), 3),
            "level": level,
            "event": event,
            "pid": os.getpid(),
            **self._fields,
            **fields,
        }
        line = orjson.dumps(record, default=str, option=orjson.OPT_NON_STR_KEYS).decode()
        stream = self._stream or sys.stdout
        stream.write(line + "\n")
        stream.flush()


def error_text(error: BaseException) -> str:
    return f"{type(error).__name__}: {error}"[:ERROR_TEXT_LIMIT]


def traceback_text(error: BaseException) -> str:
    return "".join(traceback.format_exception(error))[-TRACEBACK_LIMIT:]


def level_name(threshold: int) -> str:
    for name, value in LEVELS.items():
        if value == threshold:
            return name
    return "info"


class StdlibBridge(logging.Handler):
    def __init__(self, log: JsonLog) -> None:
        super().__init__()
        self._log = log

    def emit(self, record: logging.LogRecord) -> None:
        fields: dict[str, object] = {"logger": record.name}
        fields.update(
            (key, value) for key, value in record.__dict__.items() if key not in RECORD_ATTRIBUTES
        )
        if record.exc_info and record.exc_info[1] is not None:
            fields["error"] = error_text(record.exc_info[1])
            fields["traceback"] = traceback_text(record.exc_info[1])
        level = "error" if record.levelno >= 40 else "warning" if record.levelno >= 30 else "info"
        self._log.write(level, record.getMessage(), fields)


def install(log: JsonLog, stdlib_level: int = logging.WARNING) -> None:
    root = logging.getLogger()
    for handler in list(root.handlers):
        root.removeHandler(handler)
    root.addHandler(StdlibBridge(log))
    root.setLevel(stdlib_level)
