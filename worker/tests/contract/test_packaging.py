from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path

import pytest
import yaml

from tests.conftest import BASE_ENV, CONFIG_DIR, CONTRACT_PATH, WORKER_ROOT
from tests.contract.conftest import BACKEND_EXPORT
from worker import fetch_models as fm
from worker import settings as s
from worker.__main__ import COMMANDS

DOCKERFILE = (WORKER_ROOT / "docker" / "Dockerfile").read_text(encoding="utf-8")
UV_SYNC = (WORKER_ROOT / "docker" / "uv-sync.sh").read_text(encoding="utf-8")
PYPROJECT = tomllib.loads((WORKER_ROOT / "pyproject.toml").read_text(encoding="utf-8"))
DEV_COMPOSE = WORKER_ROOT.parent / "docker-compose-dev.yml"
IMAGE_ROOT = Path("/app")
WORKER_UID = "10001"
FLUSH_S, CLOSE_S, ENGINE_STOP_S, SPARE_S = 10, 2, 5, 10


def image_env() -> dict[str, str]:
    block = DOCKERFILE.split("\nENV PATH=", 1)[1].split("\nWORKDIR", 1)[0]
    pairs = re.findall(r"([A-Z_]+)=(\S+)", "PATH=" + block)
    return dict(pairs)


def torch_groups() -> set[str]:
    return {group["group"] for pair in PYPROJECT["tool"]["uv"]["conflicts"] for group in pair}


def test_every_flavor_is_a_locked_torch_build() -> None:
    flavors = set(re.findall(r"^\s+(\w+(?: \| \w+)*)\)", UV_SYNC, re.MULTILINE))
    named = {name.strip() for choice in flavors for name in choice.split("|")}
    assert named == torch_groups() == {"cpu", "cu126", "cu130"}
    default = re.search(r"^ARG FLAVOR=(\w+)$", DOCKERFILE, re.MULTILINE)
    assert default is not None
    assert default.group(1) in PYPROJECT["tool"]["uv"]["default-groups"]
    assert "--no-default-groups" in UV_SYNC and "--frozen" in UV_SYNC
    assert "--extra llm-local" in UV_SYNC


def test_image_contract_is_the_backend_export() -> None:
    if not BACKEND_EXPORT.is_file():
        pytest.skip(f"no backend export at {BACKEND_EXPORT}")
    exported = json.loads(BACKEND_EXPORT.read_text(encoding="utf-8"))
    assert json.loads(CONTRACT_PATH.read_text(encoding="utf-8")) == exported


def test_image_paths_match_the_shipped_config() -> None:
    settings = s.load(CONFIG_DIR, BASE_ENV)
    assert f"COPY contract/worker-contract.json {settings.worker.contract}" in DOCKERFILE
    assert f"COPY config {IMAGE_ROOT / fm.DEFAULT_CONFIG_DIR}" in DOCKERFILE
    assert f"WORKDIR {IMAGE_ROOT}" in DOCKERFILE
    assert f"/work {Path('/run/worker')}" in DOCKERFILE
    assert settings.worker.work_dir == "/work"


def test_image_env_points_libraries_at_the_models_volume() -> None:
    env = image_env()
    assert env["HF_HOME"] == "/models"
    assert env["HF_HUB_OFFLINE"] == "1"
    assert env[fm.FASTTEXT_HOME_ENV].startswith("/models/")
    assert env[fm.ROFORMER_HOME_ENV].startswith("/models/")
    assert env["PYTORCH_CUDA_ALLOC_CONF"] == "expandable_segments:True"


def test_build_label_reaches_the_producer() -> None:
    env = image_env()
    build_env = next(name for name in env if name.endswith("__BUILD"))
    settings = s.load(CONFIG_DIR, {**BASE_ENV, build_env: "2.0.1-cu126"})
    assert settings.worker.build == "2.0.1-cu126"


def test_image_runs_unprivileged_and_reports_health_through_the_cli() -> None:
    assert f"USER {WORKER_UID}:{WORKER_UID}" in DOCKERFILE
    assert f"--uid {WORKER_UID}" in DOCKERFILE
    assert "--start-period=600s" in DOCKERFILE
    assert 'CMD ["python", "-m", "worker", "health"]' in DOCKERFILE
    assert 'ENTRYPOINT ["tini", "--", "python", "-m", "worker"]' in DOCKERFILE
    assert " tini " in DOCKERFILE
    assert 'CMD ["serve"]' in DOCKERFILE
    assert {"serve", "health", "fetch-models"} <= set(COMMANDS)


def test_dev_compose_runs_the_worker_read_only_with_time_to_drain() -> None:
    worker = yaml.safe_load(DEV_COMPOSE.read_text(encoding="utf-8"))["services"]["worker"]
    assert worker["build"]["dockerfile"] == "docker/Dockerfile"
    assert worker["read_only"] is True
    assert {mount.split(":")[0] for mount in worker["tmpfs"]} == {"/tmp", "/run/worker"}
    assert {volume.split(":")[1] for volume in worker["volumes"]} == {"/models", "/work"}
    assert worker["env_file"]
    assert set(worker["environment"]) >= {"WORKER_PROFILE", "WORKER_NODE_NAME"}
    settings = s.load(CONFIG_DIR, BASE_ENV)
    grace_s = int(worker["stop_grace_period"].removesuffix("s"))
    drain_s = settings.runtime.shutdown_grace_s + FLUSH_S + CLOSE_S + ENGINE_STOP_S
    assert drain_s <= grace_s - SPARE_S
