from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from worker.domain.deadline import Deadline
from worker.domain.taste import TasteScores, TasteTraining

VERSION = "taste-202609240300-0a1b2c3d"
TOWER = VERSION + "-tower"


def scores(recall: float, ndcg: float) -> TasteScores:
    return TasteScores(recall, ndcg, 0.05, 0.3)


MODEL = scores(0.40, 0.20)
BASELINES = {
    "popularity": scores(0.20, 0.10),
    "item2vec": scores(0.18, 0.05),
    "content": scores(0.22, 0.06),
}


def trained(model: TasteScores = MODEL) -> TasteTraining:
    return TasteTraining(
        users_count=700,
        items_count=2400,
        test_users=640,
        epochs_done=10,
        evaluated_users=600,
        steps=1200,
        version=VERSION,
        tower_object=TOWER,
        model=model,
        baselines=BASELINES,
    )


@dataclass
class FakeTasteEngines:
    result: TasteTraining = field(default_factory=trained)
    failure: Exception | None = None
    calls: list[dict[str, object]] = field(default_factory=list)

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
    ) -> TasteTraining:
        deadline.check("train_taste")
        self.calls.append(
            {
                "input": input_path.read_bytes(),
                "epochs": epochs,
                "batch_size": batch_size,
                "negatives": negatives,
                "seed": seed,
                "min_users": min_users,
                "budget_s": budget_s,
                "trained_at": trained_at,
                "previous": None if previous_path is None else previous_path.read_bytes(),
            }
        )
        if self.failure is not None:
            raise self.failure
        artifact_path.write_bytes(b'{"version":"' + VERSION.encode() + b'","items":[]}')
        tower_path.write_bytes(b"tower-weights")
        return self.result
