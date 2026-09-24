from __future__ import annotations

import numpy as np
import pytest

from tests.fakes.engines import FakeEngines
from tests.unit.lyrics.conftest import lines_of, timing
from worker.domain.deadline import Deadline
from worker.domain.lyrics import gapfill, interpolate
from worker.domain.lyrics.regions import Region
from worker.observability.counters import Counters

VOCALS = np.zeros(16_000 * 120, dtype=np.float32)
TEXTS = [f"строка {index} и слова" for index in range(6)]


def test_short_runs_between_neighbours_are_interpolated_by_units() -> None:
    lines = lines_of(["раз", "два три четыре", "пять", "шесть"])
    placement = {0: timing(0, 10.0, 12.0), 3: timing(3, 20.0, 22.0)}
    added = interpolate.interpolate(lines, placement, max_gap_s=12.0)
    assert set(added) == {1, 2}
    assert added[1].start_s == 12.0 and added[1].end_s == pytest.approx(18.0)
    assert added[2].start_s == pytest.approx(18.0) and added[2].end_s == 20.0
    assert all(line.source == "interpolated" for line in added.values())
    assert [word.text for word in added[1].words] == ["два", "три", "четыре"]


def test_long_runs_wide_gaps_and_edges_are_left_alone() -> None:
    lines = lines_of(TEXTS)
    placement = {0: timing(0, 1.0, 2.0), 4: timing(4, 3.0, 4.0), 5: timing(5, 30.0, 31.0)}
    assert interpolate.interpolate(lines, placement, max_gap_s=12.0) == {}
    leading = {2: timing(2, 1.0, 2.0), 3: timing(3, 2.5, 3.0)}
    assert interpolate.interpolate(lines, leading, max_gap_s=12.0) == {}
    wide = {0: timing(0, 1.0, 2.0), 2: timing(2, 30.0, 31.0)}
    assert interpolate.interpolate(lines, wide, max_gap_s=12.0) == {}
    close = {0: timing(0, 1.0, 2.0), 2: timing(2, 13.5, 14.0)}
    assert set(interpolate.interpolate(lines, close, max_gap_s=12.0)) == {1}


async def test_gap_fill_aligns_missing_lines_between_neighbours() -> None:
    engines = FakeEngines()
    counters = Counters()
    lines = lines_of(TEXTS)
    placement = {0: timing(0, 5.0, 7.0), 3: timing(3, 30.0, 32.0), 5: timing(5, 50.0, 52.0)}
    regions = [Region(4.0, 8.0), Region(10.0, 20.0), Region(29.0, 32.0), Region(50.0, 53.0)]
    added = await gapfill.fill(
        engines,
        VOCALS,
        regions,
        lines,
        placement,
        max_gap_s=60.0,
        counters=counters,
        deadline=Deadline.after(10),
    )
    assert set(added) == {1, 2}
    assert added[1].start_s == 7.0
    assert added[2].end_s <= 30.0
    assert all(line.source == "gapfill" for line in added.values())
    assert all(line.anchor_similarity == 0.0 for line in added.values())
    assert all(line.region_score == 0.8 for line in added.values())
    assert 4 not in added
    assert counters.value("align_engine_total", engine="gapfill") == 1
    call = next(kwargs for name, kwargs in engines.calls if name == "ctc_align")
    assert call["tokens"][:3] == ["stroka", "one", "i"]


async def test_gap_fill_skips_text_the_gap_cannot_hold() -> None:
    engines = FakeEngines()
    counters = Counters()
    lines = lines_of(["длинная строка из многих букв которые никак не спеть за секунду"] * 3)
    placement = {0: timing(0, 5.0, 7.0)}
    regions = [Region(4.0, 9.0)]
    added = await gapfill.fill(
        engines,
        VOCALS[: 16_000 * 8],
        regions,
        lines,
        placement,
        max_gap_s=60.0,
        counters=counters,
        deadline=Deadline.after(10),
    )
    assert added == {}
    assert counters.value("ctc_text_too_long_total", stage="gap_fill") == 1
    assert not any(name == "ctc_align" for name, _ in engines.calls)


async def test_gap_fill_skips_long_segments_and_uses_track_edges() -> None:
    engines = FakeEngines()
    lines = lines_of(TEXTS)
    placement = {2: timing(2, 100.0, 102.0), 3: timing(3, 103.0, 104.0)}
    regions = [Region(1.0, 6.0), Region(99.0, 118.0)]
    added = await gapfill.fill(
        engines,
        VOCALS,
        regions,
        lines,
        placement,
        max_gap_s=60.0,
        counters=Counters(),
        deadline=Deadline.after(10),
    )
    assert set(added) == {4, 5}
    assert added[4].start_s >= 104.0 and added[5].end_s <= 120.0
