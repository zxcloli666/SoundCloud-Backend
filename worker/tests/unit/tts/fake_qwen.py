from __future__ import annotations

import json
import os
import queue
import struct
import sys
import threading
import time
from pathlib import Path
from typing import BinaryIO

HEAD = struct.Struct("<BII")
CHUNK = 800
CHUNKS = 3
SLOW_CHUNKS = 20
SLOW_PAUSE_S = 0.05
JOURNAL = "journal.jsonl"


def main() -> None:
    talker, codec, device = sys.argv[1:4]
    mode = Path(talker).read_text(encoding="utf-8").strip()
    note(
        "start",
        argv=[talker, codec, device],
        cwd=os.getcwd(),
        library_path=os.environ.get("LD_LIBRARY_PATH"),
        pid=os.getpid(),
    )
    out = sys.stdout.buffer
    if mode == "fail_start":
        sys.stderr.write("ggml: failed to load the talker\n")
        sys.stderr.flush()
        send(out, "E", 0, b"cannot load the talker")
        sys.exit(1)
    if mode == "deaf":
        time.sleep(60)
    send(out, "R", 0, b"16000\ncpu" if mode == "bad_rate" else b"24000\ncpu")
    frames: queue.Queue[tuple[str, int, bytes] | None] = queue.Queue()
    threading.Thread(target=pump, args=(sys.stdin.buffer, frames), daemon=True).start()
    while (frame := frames.get()) is not None:
        kind, number, payload = frame
        if kind == "S":
            note("say", id=number, payload=payload.decode())
            answer(out, mode, number, frames)
        elif kind == "C":
            note("cancel", id=number)
    note("exit")


def answer(
    out: BinaryIO, mode: str, number: int, frames: queue.Queue[tuple[str, int, bytes] | None]
) -> None:
    if mode == "stall":
        return
    if mode == "die":
        sys.stderr.write("ggml: segfault\n")
        sys.stderr.flush()
        os._exit(3)
    if mode == "refuse":
        send(out, "E", number, b"the text does not fit the context")
        return
    if mode == "garbage":
        send(out, "A", number, b"\x00\x01\x02")
        send(out, "D", number)
        return
    if mode == "slow":
        speak_slowly(out, number, frames)
        return
    for _ in range(CHUNKS):
        send(out, "A", number, struct.pack(f"<{CHUNK}f", *([0.25] * CHUNK)))
    send(out, "D", number)


def speak_slowly(
    out: BinaryIO, number: int, frames: queue.Queue[tuple[str, int, bytes] | None]
) -> None:
    for _ in range(SLOW_CHUNKS):
        send(out, "A", number, struct.pack(f"<{CHUNK}f", *([0.5] * CHUNK)))
        time.sleep(SLOW_PAUSE_S)
        if cancelled(number, frames):
            break
    send(out, "D", number)


def cancelled(number: int, frames: queue.Queue[tuple[str, int, bytes] | None]) -> bool:
    try:
        frame = frames.get_nowait()
    except queue.Empty:
        return False
    if frame is None or frame[0] != "C":
        frames.put(frame)
        return False
    note("cancel", id=frame[1])
    return frame[1] == number


def pump(stream: BinaryIO, frames: queue.Queue[tuple[str, int, bytes] | None]) -> None:
    while len(head := stream.read(HEAD.size)) == HEAD.size:
        kind, number, size = HEAD.unpack(head)
        frames.put((chr(kind), number, stream.read(size)))
    frames.put(None)


def send(out: BinaryIO, kind: str, number: int, payload: bytes = b"") -> None:
    out.write(HEAD.pack(ord(kind), number, len(payload)) + payload)
    out.flush()


def note(event: str, **fields: object) -> None:
    with Path(JOURNAL).open("a", encoding="utf-8") as journal:
        journal.write(json.dumps({"event": event, **fields}) + "\n")


if __name__ == "__main__":
    main()
