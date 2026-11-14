from __future__ import annotations

import json
import sys
from pathlib import Path

from tests.unit.tts import fake_qwen

FAKE = Path(fake_qwen.__file__)
ENGINE = "engine"
OPTIONS = {
    "program": f"{ENGINE}/scd-qwen",
    "codec": "codec.gguf",
    "voices": {
        "loli": {"talker": "loli.gguf", "speaker": "loli"},
        "moriarty": {"talker": "moriarty.gguf", "speaker": "moriarty"},
    },
}
SPOKEN = fake_qwen.CHUNK * fake_qwen.CHUNKS


def install(home: Path) -> Path:
    engine = home / ENGINE
    engine.mkdir()
    program = engine / "scd-qwen"
    program.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{FAKE}" "$@"\n')
    program.chmod(0o755)
    (home / "codec.gguf").write_text("codec")
    voice(home, "loli", "speak")
    voice(home, "moriarty", "speak")
    return home


def voice(home: Path, name: str, mode: str) -> None:
    (home / f"{name}.gguf").write_text(mode)


def journal(home: Path, event: str) -> list[dict[str, object]]:
    path = home / ENGINE / fake_qwen.JOURNAL
    lines = path.read_text().splitlines() if path.exists() else []
    return [entry for entry in map(json.loads, lines) if entry["event"] == event]
