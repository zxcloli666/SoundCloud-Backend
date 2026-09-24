from __future__ import annotations

import logging
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

from worker.domain import embedding
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.ports import BlobStore, CollabTraining, Engines
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters

LANE = "collab"
BUCKET = "COLLAB_DATA"
DIM = 128
DATASET_VERSION = 2
MIB = 1024 * 1024
MIN_SESSIONS = 2

log = logging.getLogger(__name__)


class CollabLane:
    def __init__(
        self,
        engines: Engines,
        blobs: BlobStore,
        workspace: Workspace,
        counters: Counters,
        max_object_mib: int,
        vectors_object: Callable[[str], str],
    ) -> None:
        self._engines = engines
        self._blobs = blobs
        self._workspace = workspace
        self._counters = counters
        self._max_object_bytes = max_object_mib * MIB
        self._vectors_object = vectors_object

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            job = CollabJob.parse(request)
            with self._workspace.task(f"{LANE}-{job.name}") as scratch:
                return await self._train(job, scratch, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters, **untrained())

    async def _train(self, job: CollabJob, scratch: Path, deadline: Deadline) -> Outcome:
        sessions_path = scratch / "sessions.json"
        vectors_path = scratch / "vectors.json"
        size = await self._blobs.get(BUCKET, job.name, sessions_path, deadline)
        if size > self._max_object_bytes:
            raise PermanentFailure(
                Reason.INVALID_REQUEST, f"input is {size} bytes, limit {self._max_object_bytes}"
            )
        training = await self._engines.train_collab(
            sessions_path,
            vectors_path,
            min_count=job.min_count,
            window=job.window,
            epochs=job.epochs,
            negative=job.negative,
            deadline=deadline,
        )
        log.info(
            "collab trained",
            extra={
                "object": job.name,
                "sessions": training.sessions,
                "vocab": training.vocab,
                "hr_at_20": training.hr_at_20,
                "popularity_hr_at_20": training.popularity_hr_at_20,
            },
        )
        rejection = judge(training)
        if rejection is not None:
            return rejection
        vectors_object = self._vectors_object(job.name)
        await self._blobs.put(BUCKET, vectors_object, vectors_path, deadline)
        return Outcome.ok(
            trained=True, object=vectors_object, dim=DIM, points_count=training.points_count
        )


@dataclass(frozen=True)
class CollabJob:
    name: str
    min_count: int
    window: int
    epochs: int
    negative: int

    @classmethod
    def parse(cls, request: Mapping[str, object]) -> CollabJob:
        name = request.get("object")
        if not isinstance(name, str) or not name:
            raise PermanentFailure(Reason.INVALID_REQUEST, "object must be a non-empty string")
        if request.get("dataset_version") != DATASET_VERSION or request.get("dim") != DIM:
            raise PermanentFailure(
                Reason.INVALID_REQUEST, f"expected dataset_version={DATASET_VERSION}, dim={DIM}"
            )
        return cls(
            name=name,
            min_count=positive(request, "min_count"),
            window=positive(request, "window"),
            epochs=positive(request, "epochs"),
            negative=positive(request, "negative"),
        )


def judge(training: CollabTraining) -> Outcome | None:
    if training.sessions < MIN_SESSIONS or training.vocab == 0:
        return Outcome.of(
            Reason.EMPTY_VOCAB,
            f"sessions={training.sessions} vocab={training.vocab}",
            **untrained(),
        )
    if training.hr_at_20 <= training.popularity_hr_at_20:
        return Outcome.of(
            Reason.BELOW_BASELINE,
            f"hr_at_20={training.hr_at_20:.4f} popularity={training.popularity_hr_at_20:.4f}",
            **untrained(),
        )
    return None


def untrained() -> dict[str, object]:
    return {"trained": False, "dim": DIM, "points_count": 0}


def positive(request: Mapping[str, object], key: str) -> int:
    value = request.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be an integer >= 1")
    return value
