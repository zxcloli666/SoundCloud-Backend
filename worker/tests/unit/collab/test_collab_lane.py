from __future__ import annotations

from pathlib import Path

import pytest

from tests.conftest import CONTRACT_PATH
from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.object_store import FakeBlobStore
from worker import contract as contract_module
from worker.contract import Contract
from worker.domain.collab import BUCKET, CollabLane
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Producer, Reason, Status
from worker.domain.ports import CollabTraining, EngineUnavailable
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters

INPUT = "collab-input-7c1d"
PRODUCER = Producer("test-worker", "dev", {}, None)
REQUEST: dict[str, object] = {
    "object": INPUT,
    "dataset_version": 2,
    "dim": 128,
    "min_count": 2,
    "window": 5,
    "epochs": 5,
    "negative": 10,
}


class Rig:
    def __init__(self, work_dir: Path, clock: FakeClock, max_object_mib: int = 512) -> None:
        self.work_dir = work_dir
        self.clock = clock
        self.engines = FakeEngines()
        self.blobs = FakeBlobStore()
        self.blobs.add(BUCKET, INPUT, b'{"version":2,"sessions":[[1,2],[2,3]]}')
        self.counters = Counters()
        self.lane = CollabLane(
            self.engines,
            self.blobs,
            Workspace(work_dir, self.counters),
            self.counters,
            max_object_mib,
            contract_module.load(CONTRACT_PATH).lane("collab").result_object,
        )

    async def process(self, **changes: object) -> Outcome:
        deadline = Deadline(self.clock.now() + 3600, self.clock.now)
        return await self.lane.process({**REQUEST, **changes}, deadline)


@pytest.fixture
def rig(work_dir: Path, clock: FakeClock) -> Rig:
    return Rig(work_dir, clock)


def done(outcome: Outcome) -> dict[str, object]:
    return outcome.to_done({"input_object": INPUT}, PRODUCER)


async def test_the_vectors_object_is_named_by_the_contract_template(
    work_dir: Path, clock: FakeClock, contract: Contract
) -> None:
    rig = Rig(work_dir, clock)
    rig.lane = CollabLane(
        rig.engines,
        rig.blobs,
        Workspace(work_dir, rig.counters),
        rig.counters,
        512,
        lambda name: f"{name}.renamed",
    )

    outcome = await rig.process()

    assert outcome.fields["object"] == INPUT + ".renamed"
    assert rig.blobs.puts == [(BUCKET, INPUT + ".renamed")]
    assert contract.validate("done.train_collab", done(outcome))
    assert contract.lane("collab").result_object(INPUT) == INPUT + "-vectors"


async def test_trained_vectors_are_stored_next_to_the_input(rig: Rig, contract: Contract) -> None:
    outcome = await rig.process()

    assert outcome.status is Status.OK
    assert outcome.fields == {
        "trained": True,
        "object": INPUT + "-vectors",
        "dim": 128,
        "points_count": 300,
    }
    assert rig.blobs.puts == [(BUCKET, INPUT + "-vectors")]
    assert rig.blobs.buckets[BUCKET][INPUT + "-vectors"].startswith(b'{"dim":128')
    assert contract.validate("done.train_collab", done(outcome)) == []
    method, kwargs = rig.engines.calls[0]
    assert method == "train_collab"
    assert (kwargs["min_count"], kwargs["window"], kwargs["epochs"], kwargs["negative"]) == (
        2,
        5,
        5,
        10,
    )


async def test_the_task_folder_is_removed_afterwards(rig: Rig) -> None:
    await rig.process()

    assert list(rig.work_dir.iterdir()) == []


@pytest.mark.parametrize(
    "training",
    [
        CollabTraining(500, 400, 400, 0.10, 0.12),
        CollabTraining(500, 400, 400, 0.12, 0.12),
        CollabTraining(5, 400, 400, 0.0, 0.0),
    ],
)
async def test_not_better_than_popularity_is_rejected_and_not_stored(
    rig: Rig, contract: Contract, training: CollabTraining
) -> None:
    rig.engines.collab_result = training

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.BELOW_BASELINE)
    assert outcome.fields == {"trained": False, "dim": 128, "points_count": 0}
    assert rig.blobs.puts == []
    assert contract.validate("done.train_collab", done(outcome)) == []


@pytest.mark.parametrize(
    "training", [CollabTraining(1, 0, 0, 0.0, 0.0), CollabTraining(50, 0, 0, 0.0, 0.0)]
)
async def test_too_few_sessions_or_empty_vocab_is_empty(
    rig: Rig, contract: Contract, training: CollabTraining
) -> None:
    rig.engines.collab_result = training

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.EMPTY_VOCAB)
    assert contract.validate("done.train_collab", done(outcome)) == []


async def test_missing_input_object_is_missing(rig: Rig, contract: Contract) -> None:
    outcome = await rig.process(object="collab-input-gone")

    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.OBJECT_NOT_FOUND)
    assert rig.engines.calls == []
    assert contract.validate("done.train_collab", done(outcome)) == []


async def test_unavailable_store_is_transient(rig: Rig) -> None:
    rig.blobs.unavailable = True

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.OBJECT_STORE_UNAVAILABLE)


async def test_oversized_input_is_invalid(work_dir: Path, clock: FakeClock) -> None:
    rig = Rig(work_dir, clock, max_object_mib=0)

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert rig.engines.calls == []


@pytest.mark.parametrize(
    "changes",
    [
        {"object": ""},
        {"dataset_version": 1},
        {"dim": 64},
        {"min_count": 0},
        {"window": True},
        {"epochs": "5"},
    ],
)
async def test_malformed_requests_are_invalid(rig: Rig, changes: dict[str, object]) -> None:
    outcome = await rig.process(**changes)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert outcome.fields["trained"] is False


async def test_unavailable_trainer_is_a_transient_engine_failure(rig: Rig) -> None:
    rig.engines.fail("train_collab", EngineUnavailable("train-collab", "restarting"))

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.ENGINE_CRASHED)
    assert (
        rig.counters.value("lane_engine_unavailable_total", lane="collab", slot="train-collab") == 1
    )


async def test_unexpected_errors_are_counted_internal_errors(rig: Rig) -> None:
    rig.engines.fail("train_collab", OSError("disk full"))

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INTERNAL_ERROR)
    assert rig.counters.value("lane_internal_errors_total", lane="collab", error="OSError") == 1
