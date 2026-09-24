from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path

import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.object_store import FakeBlobStore
from tests.unit.taste.fake_trainer import TOWER, VERSION, FakeTasteEngines, scores, trained
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import (
    Outcome,
    PermanentFailure,
    Producer,
    Reason,
    Status,
    TransientFailure,
)
from worker.domain.ports import EngineUnavailable
from worker.domain.taste import DATA_BUCKET, MODELS_BUCKET, TasteComparison, TasteLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import TasteSection

INPUT = "taste-input-9f2e"
PREVIOUS = "taste-202609230300-99aa88bb"
PAYLOAD = b'{"version":1}\n'
PRODUCER = Producer("test-worker", "dev", {}, None)
SETTINGS = TasteSection(max_object_mib=1, min_users=500, train_budget_s=4320)
TRAINED_AT = datetime(2026, 9, 24, 3, 0, tzinfo=UTC)
REQUEST: dict[str, object] = {
    "object": INPUT,
    "dataset_version": 1,
    "dim": 128,
    "epochs": 10,
    "batch_size": 512,
    "negatives": 1024,
    "seed": 1,
    "previous_version": None,
}


class Rig:
    def __init__(self, work_dir: Path, clock: FakeClock, settings: TasteSection = SETTINGS) -> None:
        self.work_dir = work_dir
        self.clock = clock
        self.engines = FakeTasteEngines()
        self.blobs = FakeBlobStore()
        self.blobs.add(DATA_BUCKET, INPUT, PAYLOAD)
        self.counters = Counters()
        self.lane = TasteLane(
            self.engines,
            self.blobs,
            Workspace(work_dir, self.counters),
            self.counters,
            settings,
            lambda: TRAINED_AT,
        )

    async def process(self, seconds: float = 7200, **changes: object) -> Outcome:
        deadline = Deadline(self.clock.now() + seconds, self.clock.now)
        return await self.lane.process({**REQUEST, **changes}, deadline)


@pytest.fixture
def rig(work_dir: Path, clock: FakeClock) -> Rig:
    return Rig(work_dir, clock)


def done(outcome: Outcome) -> dict[str, object]:
    return outcome.to_done({"input_object": INPUT}, PRODUCER)


async def test_a_model_above_every_baseline_is_stored_and_reported(
    rig: Rig, contract: Contract
) -> None:
    outcome = await rig.process()

    assert outcome.status is Status.OK
    assert outcome.fields == {
        "version": VERSION,
        "object": VERSION,
        "dim": 128,
        "items_count": 2400,
        "users_count": 700,
        "metrics": {
            "recall_at_50": 0.40,
            "ndcg_at_20": 0.20,
            "cold_recall_at_50": 0.05,
            "coverage_at_50": 0.3,
            "baselines": {"popularity": 0.20, "item2vec": 0.18, "content": 0.22},
        },
    }
    assert rig.blobs.puts == [(MODELS_BUCKET, TOWER), (MODELS_BUCKET, VERSION)]
    assert rig.blobs.buckets[MODELS_BUCKET][TOWER] == b"tower-weights"
    assert contract.validate("done.train_taste", done(outcome)) == []
    assert rig.counters.value("taste_verdicts_total", reason="ok") == 1


async def test_the_trainer_gets_the_request_and_the_configured_gate(rig: Rig) -> None:
    await rig.process(epochs=3, batch_size=256, negatives=64, seed=9)

    [call] = rig.engines.calls
    assert call == {
        "input": PAYLOAD,
        "epochs": 3,
        "batch_size": 256,
        "negatives": 64,
        "seed": 9,
        "min_users": 500,
        "budget_s": 4320,
        "trained_at": int(TRAINED_AT.timestamp()),
        "previous": None,
    }


async def test_the_budget_leaves_room_for_evaluation_on_a_short_deadline(rig: Rig) -> None:
    await rig.process(seconds=1000)

    assert rig.engines.calls[0]["budget_s"] == 600


async def test_the_task_folder_is_removed_afterwards(rig: Rig) -> None:
    await rig.process()

    assert list(rig.work_dir.iterdir()) == []


@pytest.mark.parametrize("test_users", [0, 499])
async def test_too_few_users_with_a_test_slice_is_empty(
    rig: Rig, contract: Contract, test_users: int
) -> None:
    rig.engines.result = replace(
        trained(), test_users=test_users, model=None, baselines=None, version=None
    )

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.TOO_FEW_USERS)
    assert outcome.fields == {"dim": 128, "items_count": 2400, "users_count": 700}
    assert rig.blobs.puts == []
    assert contract.validate("done.train_taste", done(outcome)) == []


@pytest.mark.parametrize(
    ("model", "why"),
    [
        (scores(0.23, 0.20), "recall within 5% of the best baseline"),
        (scores(0.40, 0.09), "ndcg below the popularity baseline"),
        (scores(0.0, 0.0), "nothing recalled"),
    ],
)
async def test_not_beating_the_baselines_is_rejected_and_not_stored(
    rig: Rig, contract: Contract, model: object, why: str
) -> None:
    rig.engines.result = replace(trained(), model=model)

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.BELOW_BASELINE), why
    assert rig.blobs.puts == []
    assert outcome.fields["metrics"] is not None
    assert contract.validate("done.train_taste", done(outcome)) == []
    assert rig.counters.value("taste_verdicts_total", reason="below_baseline") == 1


async def test_the_serving_version_is_handed_to_the_trainer(rig: Rig) -> None:
    rig.blobs.add(MODELS_BUCKET, PREVIOUS, b'{"version":"' + PREVIOUS.encode() + b'"}')

    await rig.process(previous_version=PREVIOUS)

    assert rig.engines.calls[0]["previous"] == b'{"version":"' + PREVIOUS.encode() + b'"}'


async def test_a_missing_serving_version_leaves_only_the_baselines(rig: Rig) -> None:
    outcome = await rig.process(previous_version=PREVIOUS)

    assert outcome.status is Status.OK
    assert rig.engines.calls[0]["previous"] is None
    assert rig.counters.value("taste_previous_total", state="missing") == 1


async def test_a_model_worse_than_the_serving_version_is_rejected(
    rig: Rig, contract: Contract
) -> None:
    rig.blobs.add(MODELS_BUCKET, PREVIOUS, b"{}")
    rig.engines.result = replace(
        trained(),
        previous_state="compared",
        previous=TasteComparison(users=300, model=scores(0.37, 0.20), previous=scores(0.53, 0.20)),
    )

    outcome = await rig.process(previous_version=PREVIOUS)

    assert (outcome.status, outcome.reason) == (Status.REJECTED, Reason.BELOW_BASELINE)
    assert outcome.detail is not None and f"previous_version={PREVIOUS}" in outcome.detail
    assert rig.blobs.puts == []
    assert contract.validate("done.train_taste", done(outcome)) == []
    assert rig.counters.value("taste_previous_total", state="compared") == 1


async def test_a_model_close_to_the_serving_version_passes(rig: Rig) -> None:
    rig.blobs.add(MODELS_BUCKET, PREVIOUS, b"{}")
    rig.engines.result = replace(
        trained(),
        previous=TasteComparison(users=300, model=scores(0.51, 0.20), previous=scores(0.53, 0.20)),
    )

    outcome = await rig.process(previous_version=PREVIOUS)

    assert outcome.status is Status.OK


async def test_too_few_evaluated_users_is_empty_whatever_the_timed_count(rig: Rig) -> None:
    rig.engines.result = replace(trained(), evaluated_users=120)

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.TOO_FEW_USERS)
    assert outcome.detail is not None and "evaluated_users=120" in outcome.detail
    assert rig.blobs.puts == []


async def test_a_model_that_never_stepped_is_not_published(rig: Rig, contract: Contract) -> None:
    rig.engines.result = replace(
        trained(), model=None, baselines=None, version=None, steps=0, budget_spent=True
    )

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)
    assert rig.blobs.puts == []
    assert contract.validate("done.train_taste", done(outcome)) == []


async def test_a_tower_without_its_artifact_is_removed(rig: Rig) -> None:
    rig.blobs.refused.add(VERSION)

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.OBJECT_STORE_UNAVAILABLE)
    assert rig.blobs.deletes == [(MODELS_BUCKET, TOWER)]
    assert TOWER not in rig.blobs.buckets[MODELS_BUCKET]


async def test_exactly_five_percent_above_the_best_baseline_passes(rig: Rig) -> None:
    rig.engines.result = trained(model=scores(1.05 * 0.22, 0.10))

    outcome = await rig.process()

    assert outcome.status is Status.OK


async def test_missing_input_object_is_missing(rig: Rig, contract: Contract) -> None:
    outcome = await rig.process(object="taste-input-gone")

    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.OBJECT_NOT_FOUND)
    assert rig.engines.calls == []
    assert contract.validate("done.train_taste", done(outcome)) == []


async def test_unavailable_store_is_transient(rig: Rig, contract: Contract) -> None:
    rig.blobs.unavailable = True

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.OBJECT_STORE_UNAVAILABLE)
    assert contract.validate("done.train_taste", done(outcome)) == []


async def test_oversized_input_is_invalid(work_dir: Path, clock: FakeClock) -> None:
    rig = Rig(work_dir, clock)
    rig.blobs.add(DATA_BUCKET, INPUT, b"x" * (1024 * 1024 + 1))

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert rig.engines.calls == []


@pytest.mark.parametrize(
    "changes",
    [
        {"object": ""},
        {"dataset_version": 2},
        {"dim": 64},
        {"epochs": 0},
        {"batch_size": True},
        {"negatives": "5"},
        {"seed": None},
        {"previous_version": 3},
    ],
)
async def test_malformed_requests_are_invalid(
    rig: Rig, contract: Contract, changes: dict[str, object]
) -> None:
    outcome = await rig.process(**changes)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.INVALID_REQUEST)
    assert outcome.fields == {"dim": 128}
    assert contract.validate("done.train_taste", done(outcome)) == []


@pytest.mark.parametrize(
    ("failure", "reason"),
    [
        (EngineUnavailable("train-taste", "restarting"), Reason.ENGINE_CRASHED),
        (TransientFailure(Reason.DEADLINE_EXCEEDED, "slot=train-taste"), Reason.DEADLINE_EXCEEDED),
        (TransientFailure(Reason.OUT_OF_MEMORY, "slot=train-taste"), Reason.OUT_OF_MEMORY),
        (PermanentFailure(Reason.INVALID_REQUEST, "bad header"), Reason.INVALID_REQUEST),
        (PermanentFailure(Reason.MODEL_OUTPUT_INVALID, "nan"), Reason.MODEL_OUTPUT_INVALID),
        (OSError("disk full"), Reason.INTERNAL_ERROR),
    ],
)
async def test_trainer_failures_become_contract_failures(
    rig: Rig, contract: Contract, failure: Exception, reason: Reason
) -> None:
    rig.engines.failure = failure

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, reason)
    assert rig.blobs.puts == []
    assert contract.validate("done.train_taste", done(outcome)) == []
    assert reason.value in contract.lane("taste").worker_reasons


async def test_a_trained_result_without_a_valid_version_is_invalid_output(rig: Rig) -> None:
    rig.engines.result = replace(trained(), version="v1")

    outcome = await rig.process()

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.MODEL_OUTPUT_INVALID)
    assert rig.blobs.puts == []


async def test_an_expired_deadline_is_deadline_exceeded(rig: Rig) -> None:
    outcome = await rig.process(seconds=0)

    assert (outcome.status, outcome.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)
