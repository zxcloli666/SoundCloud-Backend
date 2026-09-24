from __future__ import annotations

import json
import time
from pathlib import Path

import pytest
import torch

from tests.unit.taste.synthetic import World, write_input
from worker.domain.taste import judge
from worker.engines import taste_training
from worker.models.taste_trainer import TasteTrainerSlot
from worker.runtime.protocol import SlotSpec

pytestmark = pytest.mark.models

WORLD = World(users=3000, tracks=20_000, tastes=40, seed=11)
SPEC = SlotSpec("train-taste", "worker.models.taste_trainer:TasteTrainerSlot", "", "", "cuda", 1, 0)
MIN_USERS = 500
BUDGET_S = 900
LIBRARY_WORKSPACE_BYTES = 32 * 1024 * 1024


def test_the_gpu_trainer_beats_every_baseline_on_a_planted_taste(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", WORLD)
    slot = TasteTrainerSlot()
    slot.load(SPEC)
    slot.warmup()
    started = time.monotonic()
    try:
        _, result = slot.invoke(
            "train",
            {},
            {
                "input_path": str(source),
                "artifact_path": str(tmp_path / "artifact.json"),
                "tower_path": str(tmp_path / "tower.safetensors"),
                "epochs": 10,
                "batch_size": 512,
                "negatives": 1024,
                "seed": 1,
                "min_users": MIN_USERS,
                "budget_s": BUDGET_S,
                "trained_at": 1_790_000_000,
            },
        )
    finally:
        slot.unload()
        torch.cuda.empty_cache()
    seconds = time.monotonic() - started
    metrics = result["metrics"]
    assert isinstance(metrics, dict)
    print(json.dumps({"seconds": round(seconds, 1), **result}, indent=1))

    training = taste_training(result)
    assert training.epochs_done == 10
    assert training.evaluated_users >= MIN_USERS
    assert training.items_count == WORLD.tracks
    assert judge(training, MIN_USERS) is None
    assert torch.cuda.memory_allocated() < LIBRARY_WORKSPACE_BYTES
