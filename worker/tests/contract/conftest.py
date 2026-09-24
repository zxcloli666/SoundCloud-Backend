from __future__ import annotations

from pathlib import Path

import pytest

from tests.conftest import CONTRACT_PATH, WORKER_ROOT
from worker import contract as contract_module
from worker.contract import Contract

BACKEND_EXPORT = WORKER_ROOT.parent / "backend-contracts" / "contract" / "worker-contract.json"


def wire_contract_path() -> Path:
    return BACKEND_EXPORT if BACKEND_EXPORT.is_file() else CONTRACT_PATH


@pytest.fixture(scope="session")
def contract() -> Contract:
    return contract_module.load(wire_contract_path())
