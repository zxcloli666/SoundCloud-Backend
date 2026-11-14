from __future__ import annotations

import struct
from dataclasses import dataclass
from typing import IO

import numpy as np
from numpy.typing import NDArray

SAY = ord("S")
CANCEL = ord("C")
READY = ord("R")
AUDIO = ord("A")
DONE = ord("D")
FAILED = ord("E")
HEAD = struct.Struct("<BII")
MAX_PAYLOAD = 16 << 20
SAMPLE = np.dtype("<f4")


class BadFrame(Exception):
    pass


@dataclass(frozen=True)
class Frame:
    kind: int
    id: int
    payload: bytes = b""


def encode(frame: Frame) -> bytes:
    return HEAD.pack(frame.kind, frame.id, len(frame.payload)) + frame.payload


def read(stream: IO[bytes]) -> Frame | None:
    head = stream.read(HEAD.size)
    if not head:
        return None
    if len(head) < HEAD.size:
        raise BadFrame("the stream ended inside a frame head")
    kind, number, size = HEAD.unpack(head)
    if size > MAX_PAYLOAD:
        raise BadFrame(f"a frame of {size} bytes is over {MAX_PAYLOAD}")
    payload = stream.read(size)
    if len(payload) < size:
        raise BadFrame("the stream ended inside a frame")
    return Frame(kind, number, payload)


def samples(payload: bytes) -> NDArray[np.float32]:
    if len(payload) % SAMPLE.itemsize:
        raise BadFrame(f"audio of {len(payload)} bytes is not whole float32 samples")
    return np.frombuffer(payload, SAMPLE).astype(np.float32)


def text(payload: bytes, limit: int) -> str:
    return payload.decode(errors="replace").strip()[:limit]
