from __future__ import annotations

import numpy as np

from tests.fakes.engines import FakeEngines
from tests.unit.lyrics.conftest import RUSSIAN_LINES, lines_of
from worker.domain.deadline import Deadline
from worker.domain.lyrics import rescue
from worker.observability.counters import Counters


async def test_global_ctc_keeps_the_acoustic_score_out_of_the_text_similarity() -> None:
    placement, score = await rescue.global_ctc(
        FakeEngines(),
        np.zeros(16_000 * 40, dtype=np.float32),
        lines_of(RUSSIAN_LINES),
        Counters(),
        Deadline.after(10),
    )
    assert score == 0.8
    assert len(placement) == len(RUSSIAN_LINES)
    assert {timing.region_score for timing in placement.values()} == {0.8}
    assert {timing.anchor_similarity for timing in placement.values()} == {0.0}
