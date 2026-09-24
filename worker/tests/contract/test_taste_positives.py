from __future__ import annotations

import orjson

from tests.conftest import WORKER_ROOT
from worker.models.taste_trainer import Split, read_dataset

FIXTURE = WORKER_ROOT.parent / "backend-contracts" / "fixtures" / "taste" / "positives.jsonl"


def test_the_worker_counts_timed_positives_like_the_jobs_export_gate() -> None:
    header = orjson.loads(FIXTURE.read_bytes().splitlines()[0])

    split = Split.of(read_dataset(FIXTURE), min_users=1)

    assert split.test_users == header["users_with_5_timed_positives"] == 4
