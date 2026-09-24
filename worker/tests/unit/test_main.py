from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HEAVY_MODULES = ("worker.app", "worker.fetch_models", "nagisa", "aiohttp", "nats", "torch")
PROBE = """
import sys
from worker.__main__ import main
code = main(["health", "--path", sys.argv[1]])
heavy = [name for name in sys.argv[2:] if name in sys.modules]
print(code, *heavy)
"""


def test_health_command_does_not_import_the_application(tmp_path: Path) -> None:
    finished = subprocess.run(
        [sys.executable, "-c", PROBE, str(tmp_path / "missing.json"), *HEAVY_MODULES],
        capture_output=True,
        text=True,
        check=True,
    )

    assert finished.stdout.splitlines()[-1] == "1"
