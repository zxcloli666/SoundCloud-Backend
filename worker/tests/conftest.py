from __future__ import annotations

import os
from collections.abc import Iterator
from pathlib import Path

import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.jetstream import FakeNats
from tests.fakes.object_store import FakeBlobStore
from worker import contract as contract_module
from worker import settings as settings_module
from worker.contract import Contract
from worker.settings import Settings

WORKER_ROOT = Path(__file__).resolve().parent.parent
CONFIG_DIR = WORKER_ROOT / "config"
CONTRACT_PATH = WORKER_ROOT / "contract" / "worker-contract.json"

BASE_ENV = {
    "WORKER_NODE_NAME": "test-worker",
    "NATS_URL": "nats://127.0.0.1:4222",
    "NATS_USER": "worker-trusted",
    "NATS_PASSWORD": "secret",
}
LLM_KEYS = {
    "ANTHROPIC_API_KEY": "sk-test",
    "LLM_FALLBACK_KEY": "sk-test",
    "LLM_FALLBACK_URL": "https://llm.test/v1/chat/completions",
    "LLM_FALLBACK_MODEL": "test-model",
}


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    skips = {
        "integration": (
            "NATS_TEST_URL" not in os.environ,
            "needs a live NATS (NATS_TEST_URL)",
        ),
        "models": (not cuda_available(), "needs CUDA and cached weights"),
        "eval": ("EVAL_DATA_DIR" not in os.environ, "needs the eval manifest (EVAL_DATA_DIR)"),
    }
    for item in items:
        for marker, (skip, reason) in skips.items():
            if item.get_closest_marker(marker) and skip:
                item.add_marker(pytest.mark.skip(reason=reason))


def cuda_available() -> bool:
    return Path("/dev/nvidia0").exists() and "WORKER_TEST_NO_GPU" not in os.environ


@pytest.fixture(scope="session")
def contract() -> Contract:
    return contract_module.load(CONTRACT_PATH)


@pytest.fixture
def base_env() -> dict[str, str]:
    return dict(BASE_ENV)


@pytest.fixture
def settings(base_env: dict[str, str]) -> Settings:
    return settings_module.load(CONFIG_DIR, base_env)


@pytest.fixture
def clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def fake_nats(clock: FakeClock, contract: Contract) -> FakeNats:
    bus = FakeNats(clock)
    provision_like_jobs(bus, contract)
    return bus


@pytest.fixture
def engines() -> FakeEngines:
    return FakeEngines()


@pytest.fixture
def blobs() -> FakeBlobStore:
    return FakeBlobStore()


@pytest.fixture
def work_dir(tmp_path: Path) -> Iterator[Path]:
    path = tmp_path / "work"
    path.mkdir()
    yield path


def provision_like_jobs(bus: FakeNats, contract: Contract) -> None:
    from nats.js import api

    for name, stream in contract.streams.items():
        bus.provision_stream(
            name,
            stream.subjects,
            retention=stream.retention,
            max_age_s=stream.max_age_s,
            duplicate_window_s=stream.duplicate_window_s,
        )
    for lane in contract.lanes.values():
        bus.provision_consumer(
            lane.stream,
            api.ConsumerConfig(
                durable_name=lane.durable,
                filter_subject=lane.filter_subject,
                ack_policy=api.AckPolicy.EXPLICIT,
                deliver_policy=api.DeliverPolicy.ALL,
                replay_policy=api.ReplayPolicy.INSTANT,
                ack_wait=lane.ack_wait_s,
                max_deliver=lane.max_deliver,
                max_ack_pending=lane.max_ack_pending,
            ),
        )
    for bucket in contract.object_stores:
        bus.provision_object_store(bucket)
