from __future__ import annotations

import argparse
import asyncio
import os
import sys
from pathlib import Path

from worker import health
from worker.observability.health_file import HEALTH_PATH

COMMANDS = ("serve", "health", "fetch-models")
DEFAULT_CONFIG_DIR = Path("config")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="worker")
    parser.add_argument("command", choices=COMMANDS)
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    parsed = parser.parse_args(argv)
    if parsed.command == "health":
        return health.main(parsed.arguments)
    if parsed.command == "fetch-models":
        from worker import fetch_models

        return fetch_models.main(parsed.arguments)
    return serve(parsed.arguments)


def serve(argv: list[str]) -> int:
    from worker import app
    from worker import contract as contract_module
    from worker import settings as settings_module
    from worker.bus.consumers import EX_CONFIG
    from worker.contract import ContractError
    from worker.observability.logging import JsonLog
    from worker.settings import SettingsError

    parser = argparse.ArgumentParser(prog="worker serve")
    parser.add_argument("--config-dir", type=Path, default=DEFAULT_CONFIG_DIR)
    parser.add_argument("--health-file", type=Path, default=HEALTH_PATH)
    parsed = parser.parse_args(argv)
    log = JsonLog()
    try:
        settings = settings_module.load(parsed.config_dir, os.environ)
        contract = contract_module.load(Path(settings.worker.contract))
    except (SettingsError, ContractError, OSError) as error:
        log.error("startup_config_error", error=f"{type(error).__name__}: {error}")
        return EX_CONFIG
    try:
        return asyncio.run(app.run(settings, contract, health_path=parsed.health_file))
    except Exception as error:
        log.exception("serve_crashed", error)
        return app.EXIT_CRASHED


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
