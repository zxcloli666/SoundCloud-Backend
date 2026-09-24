from __future__ import annotations

import logging
import re
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Protocol

from worker.domain import embedding
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.ports import BlobStore
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import TasteSection

LANE = "taste"
DATA_BUCKET = "TASTE_DATA"
MODELS_BUCKET = "TASTE_MODELS"
DIM = 128
DATASET_VERSION = 1
MIB = 1024 * 1024
BASELINE_MARGIN = 1.05
PREVIOUS_SHARE = 0.95
BUDGET_SHARE = 0.6
PREVIOUS_MISSING = "missing"
BASELINES = ("popularity", "item2vec", "content")
VERSION_PATTERN = re.compile(r"^taste-[0-9]{12}-[0-9a-f]{8}$")

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class TasteScores:
    recall_at_50: float
    ndcg_at_20: float
    cold_recall_at_50: float
    coverage_at_50: float


@dataclass(frozen=True)
class TasteComparison:
    users: int
    model: TasteScores
    previous: TasteScores


@dataclass(frozen=True)
class TasteTraining:
    users_count: int
    items_count: int
    test_users: int
    epochs_done: int
    evaluated_users: int = 0
    steps: int = 0
    budget_spent: bool = False
    previous_state: str = "none"
    version: str | None = None
    tower_object: str | None = None
    model: TasteScores | None = None
    baselines: Mapping[str, TasteScores] | None = None
    previous: TasteComparison | None = None


class TasteEngines(Protocol):
    async def train_taste(
        self,
        input_path: Path,
        artifact_path: Path,
        tower_path: Path,
        *,
        previous_path: Path | None,
        epochs: int,
        batch_size: int,
        negatives: int,
        seed: int,
        min_users: int,
        budget_s: int,
        trained_at: int,
        deadline: Deadline,
    ) -> TasteTraining: ...


class TasteLane:
    def __init__(
        self,
        engines: TasteEngines,
        blobs: BlobStore,
        workspace: Workspace,
        counters: Counters,
        settings: TasteSection,
        wall_clock: Callable[[], datetime] = lambda: datetime.now(UTC),
    ) -> None:
        self._engines = engines
        self._blobs = blobs
        self._workspace = workspace
        self._counters = counters
        self._settings = settings
        self._wall_clock = wall_clock

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            job = TasteJob.parse(request)
            with self._workspace.task(f"{LANE}-{job.name}") as scratch:
                return await self._train(job, scratch, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters, dim=DIM)

    async def _train(self, job: TasteJob, scratch: Path, deadline: Deadline) -> Outcome:
        input_path = scratch / "taste-input.jsonl"
        artifact_path = scratch / "artifact.json"
        tower_path = scratch / "tower.safetensors"
        limit = self._settings.max_object_mib * MIB
        size = await self._blobs.get(DATA_BUCKET, job.name, input_path, deadline)
        if size > limit:
            raise PermanentFailure(Reason.INVALID_REQUEST, f"input is {size} bytes, limit {limit}")
        previous_path = await self._previous(job, scratch / "previous.json", deadline)
        training = await self._engines.train_taste(
            input_path,
            artifact_path,
            tower_path,
            previous_path=previous_path,
            epochs=job.epochs,
            batch_size=job.batch_size,
            negatives=job.negatives,
            seed=job.seed,
            min_users=self._settings.min_users,
            budget_s=self._budget(deadline),
            trained_at=int(self._wall_clock().timestamp()),
            deadline=deadline,
        )
        if job.previous_version is not None and previous_path is not None:
            self._counters.inc("taste_previous_total", state=training.previous_state)
        log.info("taste trained", extra=trained_extra(job, training))
        verdict = judge(training, self._settings.min_users, job.previous_version)
        if verdict is not None:
            self._counters.inc("taste_verdicts_total", reason=str(verdict.reason))
            return verdict
        version, tower_object = training.version, training.tower_object
        if version is None or tower_object is None or not VERSION_PATTERN.fullmatch(version):
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"version={version!r}")
        await self._blobs.put(MODELS_BUCKET, tower_object, tower_path, deadline)
        try:
            await self._blobs.put(MODELS_BUCKET, version, artifact_path, deadline)
        except Exception:
            await self._forget_tower(tower_object, deadline)
            raise
        self._counters.inc("taste_verdicts_total", reason="ok")
        return Outcome.ok(
            version=version, object=version, **counts(training), metrics=wire_metrics(training)
        )

    async def _previous(self, job: TasteJob, path: Path, deadline: Deadline) -> Path | None:
        if job.previous_version is None:
            return None
        try:
            await self._blobs.get(MODELS_BUCKET, job.previous_version, path, deadline)
        except PermanentFailure as error:
            if error.reason not in (Reason.OBJECT_NOT_FOUND, Reason.INVALID_REQUEST):
                raise
            self._counters.inc("taste_previous_total", state=PREVIOUS_MISSING)
            log.warning(
                "previous taste version is not readable; the gate compares with baselines only",
                extra={"previous_version": job.previous_version, "reason": str(error.reason)},
            )
            return None
        return path

    async def _forget_tower(self, tower_object: str, deadline: Deadline) -> None:
        try:
            await self._blobs.delete(MODELS_BUCKET, tower_object, deadline)
        except Exception as error:
            self._counters.inc("taste_orphan_towers_total")
            log.warning(
                "taste tower is left without its artifact",
                extra={"object": tower_object, "error": str(error)},
            )

    def _budget(self, deadline: Deadline) -> int:
        share = int(BUDGET_SHARE * deadline.remaining())
        return max(1, min(self._settings.train_budget_s, share))


@dataclass(frozen=True)
class TasteJob:
    name: str
    epochs: int
    batch_size: int
    negatives: int
    seed: int
    previous_version: str | None

    @classmethod
    def parse(cls, request: Mapping[str, object]) -> TasteJob:
        name = request.get("object")
        if not isinstance(name, str) or not name:
            raise PermanentFailure(Reason.INVALID_REQUEST, "object must be a non-empty string")
        if request.get("dataset_version") != DATASET_VERSION or request.get("dim") != DIM:
            raise PermanentFailure(
                Reason.INVALID_REQUEST, f"expected dataset_version={DATASET_VERSION}, dim={DIM}"
            )
        previous = request.get("previous_version")
        if previous is not None and not isinstance(previous, str):
            raise PermanentFailure(Reason.INVALID_REQUEST, "previous_version must be string|null")
        return cls(
            name=name,
            epochs=positive(request, "epochs"),
            batch_size=positive(request, "batch_size"),
            negatives=positive(request, "negatives"),
            seed=positive(request, "seed"),
            previous_version=previous,
        )


def judge(
    training: TasteTraining, min_users: int, previous_version: str | None = None
) -> Outcome | None:
    enough = min(training.test_users, training.evaluated_users) >= min_users
    if training.budget_spent and enough:
        return Outcome.of(
            Reason.DEADLINE_EXCEEDED, "training budget ran out before the first step", dim=DIM
        )
    if not enough or training.model is None or not training.baselines:
        return Outcome.of(
            Reason.TOO_FEW_USERS,
            f"users_with_5_timed_positives={training.test_users} "
            f"evaluated_users={training.evaluated_users} min={min_users}",
            **counts(training),
        )
    return below_baselines(training) or below_previous(training, previous_version)


def below_baselines(training: TasteTraining) -> Outcome | None:
    model, baselines = training.model, training.baselines
    if model is None or not baselines:
        return None
    best_recall = max(scores.recall_at_50 for scores in baselines.values())
    best_ndcg = max(scores.ndcg_at_20 for scores in baselines.values())
    if (
        model.recall_at_50 <= 0.0
        or model.recall_at_50 < BASELINE_MARGIN * best_recall
        or model.ndcg_at_20 < best_ndcg
    ):
        return Outcome.of(
            Reason.BELOW_BASELINE,
            f"recall_at_50={model.recall_at_50:.4f} best_baseline={best_recall:.4f} "
            f"ndcg_at_20={model.ndcg_at_20:.4f} best_baseline={best_ndcg:.4f}",
            **counts(training),
            metrics=wire_metrics(training),
        )
    return None


def below_previous(training: TasteTraining, previous_version: str | None) -> Outcome | None:
    comparison = training.previous
    if comparison is None:
        return None
    model, previous = comparison.model, comparison.previous
    if (
        model.recall_at_50 >= PREVIOUS_SHARE * previous.recall_at_50
        and model.ndcg_at_20 >= PREVIOUS_SHARE * previous.ndcg_at_20
    ):
        return None
    return Outcome.of(
        Reason.BELOW_BASELINE,
        f"previous_version={previous_version} fresh_users={comparison.users} "
        f"recall_at_50={model.recall_at_50:.4f} previous={previous.recall_at_50:.4f} "
        f"ndcg_at_20={model.ndcg_at_20:.4f} previous={previous.ndcg_at_20:.4f}",
        **counts(training),
        metrics=wire_metrics(training),
    )


def wire_metrics(training: TasteTraining) -> dict[str, object]:
    model, baselines = training.model, training.baselines
    if model is None or baselines is None:
        raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, "trained model has no metrics")
    return {
        "recall_at_50": model.recall_at_50,
        "ndcg_at_20": model.ndcg_at_20,
        "cold_recall_at_50": model.cold_recall_at_50,
        "coverage_at_50": model.coverage_at_50,
        "baselines": {name: baselines[name].recall_at_50 for name in BASELINES},
    }


def counts(training: TasteTraining) -> dict[str, object]:
    return {
        "dim": DIM,
        "items_count": training.items_count,
        "users_count": training.users_count,
    }


def trained_extra(job: TasteJob, training: TasteTraining) -> dict[str, object]:
    extra: dict[str, object] = {
        "object": job.name,
        "previous_version": job.previous_version,
        "users": training.users_count,
        "items": training.items_count,
        "test_users": training.test_users,
        "evaluated_users": training.evaluated_users,
        "epochs_done": training.epochs_done,
        "steps": training.steps,
        "previous_state": training.previous_state,
        "version": training.version,
    }
    if training.model is not None and training.baselines is not None:
        extra["recall_at_50"] = training.model.recall_at_50
        extra["ndcg_at_20"] = training.model.ndcg_at_20
        for name, scores in training.baselines.items():
            extra[f"{name}_recall_at_50"] = scores.recall_at_50
    if training.previous is not None:
        extra["previous_recall_at_50"] = training.previous.previous.recall_at_50
        extra["fresh_recall_at_50"] = training.previous.model.recall_at_50
    return extra


def positive(request: Mapping[str, object], key: str) -> int:
    value = request.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be an integer >= 1")
    return value
