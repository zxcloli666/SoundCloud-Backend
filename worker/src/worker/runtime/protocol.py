from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Protocol

import numpy as np


class BadInput(Exception):
    pass


class ErrorKind(StrEnum):
    OOM = "oom"
    BAD_INPUT = "bad_input"
    MODEL_ERROR = "model_error"


class CommandKind(StrEnum):
    PING = "ping"
    LOAD = "load"
    UNLOAD = "unload"
    STOP = "stop"


@dataclass(frozen=True)
class ArrayRef:
    shm_name: str
    shape: tuple[int, ...]
    dtype: str


@dataclass(frozen=True)
class SlotSpec:
    name: str
    loader: str
    model: str
    revision: str
    device: str
    max_batch: int
    max_wait_ms: int
    options: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class Call:
    id: int
    slot: str
    method: str
    deadline_at: float
    arrays: Mapping[str, ArrayRef] = field(default_factory=dict)
    args: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class Command:
    id: int
    kind: CommandKind
    slot: str | None = None


@dataclass(frozen=True)
class Reply:
    id: int
    arrays: Mapping[str, ArrayRef] = field(default_factory=dict)
    result: Mapping[str, object] = field(default_factory=dict)
    error_kind: ErrorKind | None = None
    error: str | None = None
    duration_ms: float = 0.0

    @property
    def ok(self) -> bool:
        return self.error_kind is None


@dataclass(frozen=True)
class SlotState:
    slot: str
    loaded: bool
    calls: int
    reserved_mib: int
    allocated_mib: int


@dataclass(frozen=True)
class Pong:
    id: int
    slots: tuple[SlotState, ...]


Arrays = Mapping[str, np.ndarray]


class ModelSlot(Protocol):
    def load(self, spec: SlotSpec) -> None: ...

    def warmup(self) -> None: ...

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]: ...

    def unload(self) -> None: ...
