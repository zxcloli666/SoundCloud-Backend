from __future__ import annotations

import json
import subprocess
from pathlib import Path

TS_SYNC_BYTE = 0x47


def make_mpegts(path: Path, seconds: int = 7) -> bytes:
    argv = [
        "ffmpeg", "-nostdin", "-v", "error", "-y",
        "-f", "lavfi", "-i", f"testsrc2=size=320x180:rate=25:duration={seconds}",
        "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}",
        "-c:v", "libx264", "-g", "100", "-pix_fmt", "yuv420p", "-c:a", "aac",
        "-f", "mpegts", str(path),
    ]  # fmt: skip
    subprocess.run(argv, check=True)
    blob = path.read_bytes()
    assert blob[0] == TS_SYNC_BYTE
    return blob


def stream_types(path: Path) -> list[str]:
    out = ffprobe(path, ["-show_entries", "stream=codec_type"])
    return [stream["codec_type"] for stream in out["streams"]]


def keyframe_times(path: Path) -> list[float]:
    out = ffprobe(
        path,
        ["-select_streams", "v:0", "-show_entries", "packet=pts_time,flags"],
    )
    return [float(packet["pts_time"]) for packet in out["packets"] if "K" in packet["flags"]]


def top_level_boxes(blob: bytes) -> list[str]:
    boxes, at = [], 0
    while at + 8 <= len(blob):
        size = int.from_bytes(blob[at : at + 4], "big")
        boxes.append(blob[at + 4 : at + 8].decode("ascii"))
        if size < 8:
            break
        at += size
    return boxes


def ffprobe(path: Path, args: list[str]) -> dict:
    argv = ["ffprobe", "-v", "error", *args, "-of", "json", str(path)]
    return json.loads(subprocess.run(argv, check=True, capture_output=True).stdout)
