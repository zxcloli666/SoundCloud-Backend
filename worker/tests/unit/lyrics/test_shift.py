from __future__ import annotations

from tests.unit.lyrics.conftest import lines_of, timing
from worker.domain.lyrics import lrc, text
from worker.domain.lyrics.placement import LineTiming, shifted

PAUSE_S = 2.5
LIMIT_S = 20.0


def placement() -> dict[int, LineTiming]:
    return {0: timing(0, 1.0, 2.0), 1: timing(1, 5.0, 6.0), 2: timing(2, 19.8, 20.0)}


def rendered_with(shift_s: float) -> lrc.Rendered:
    raw = "Первая строка\nВторая строка\nТретья строка"
    moved = shifted(placement(), shift_s, LIMIT_S)
    return lrc.render(text.parse(raw), lines_of(raw.split("\n")), moved, pause_s=PAUSE_S)


def test_zero_shift_changes_nothing() -> None:
    assert shifted(placement(), 0.0, LIMIT_S) == placement()


def test_words_and_lines_move_together() -> None:
    before = rendered_with(0.0)
    after = rendered_with(0.04)
    stamps = after.synced_lrc.split("\n")
    assert "[00:01.04]Первая строка" in stamps
    assert "[00:05.04]Вторая строка" in stamps
    for old, new in zip(before.words[:6], after.words[:6], strict=True):
        assert new["start_ms"] == old["start_ms"] + 40
        assert new["end_ms"] == old["end_ms"] + 40


def test_shift_clamps_at_the_end() -> None:
    moved = shifted(placement(), 0.5, LIMIT_S)[2]
    assert moved.end_s == LIMIT_S
    assert moved.words[-1].end_s == LIMIT_S
    assert all(word.start_s <= LIMIT_S for word in moved.words)


def test_shift_clamps_at_zero() -> None:
    moved = shifted(placement(), -3.0, LIMIT_S)[0]
    assert moved.start_s == 0.0
    assert all(word.start_s >= 0.0 and word.end_s >= 0.0 for word in moved.words)
