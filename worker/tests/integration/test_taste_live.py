from __future__ import annotations

import json
from pathlib import Path

import pytest

from tests.integration.test_end_to_end import (
    Live,
    all_lanes_serving,
    fixtures,
    live,
    start_serve,
    wait_for_dones,
    wait_for_status,
)
from tests.unit.taste.synthetic import Mode, World, synthetic_input

pytestmark = pytest.mark.integration

__all__ = ["fixtures", "live"]

LANES = ("taste",)
PLANTED = World(users=560, tracks=1600)
INPUTS = {
    "taste-input-planted": PLANTED,
    "taste-input-popular": World(users=560, tracks=1600, mode=Mode.POPULARITY),
    "taste-input-small": World(users=120, tracks=400),
}
EXPECTED = {
    "taste-input-planted": ("ok", None),
    "taste-input-popular": ("rejected", "below_baseline"),
    "taste-input-small": ("empty", "too_few_users"),
    "taste-input-gone": ("missing", "object_not_found"),
}


async def test_the_taste_lane_answers_every_verdict_over_nats(
    live: Live, fixtures: object, tmp_path: Path
) -> None:
    store = await live.js.object_store("TASTE_DATA")
    for name, world in INPUTS.items():
        await store.put(name, synthetic_input(world))
    worker = await start_serve(live, fixtures, tmp_path, LANES, WORKER__RUNTIME__DEVICE="cpu")
    try:
        await wait_for_status(worker, all_lanes_serving(LANES))
        for name in EXPECTED:
            await live.publish("train.taste.new", task(name), {"Nats-Msg-Id": f"taste:{name}"})
        dones = await wait_for_dones(live, worker, len(EXPECTED))
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    by_input = {}
    for subject, done in dones:
        assert subject == "done.train_taste"
        assert live.contract.validate(subject, done) == [], done
        by_input[done["input_object"]] = done
    assert {name: (done["status"], done.get("reason")) for name, done in by_input.items()} == (
        EXPECTED
    )
    trained = by_input["taste-input-planted"]
    assert trained["object"] == trained["version"]
    assert trained["items_count"] == PLANTED.tracks
    assert trained["metrics"]["recall_at_50"] > max(trained["metrics"]["baselines"].values())
    models = await live.js.object_store("TASTE_MODELS")
    artifact = json.loads((await models.get(trained["version"])).data or b"{}")
    tower = await models.get_info(f"{trained['version']}-tower")
    assert artifact["version"] == trained["version"]
    assert len(artifact["items"]) == PLANTED.tracks
    assert (tower.size or 0) > 0
    assert await live.messages_left("TRAIN_TASTE") == 0


def task(name: str) -> dict[str, object]:
    return {
        "object": name,
        "dataset_version": 1,
        "dim": 128,
        "epochs": 4,
        "batch_size": 256,
        "negatives": 256,
        "seed": 1,
        "previous_version": None,
    }
