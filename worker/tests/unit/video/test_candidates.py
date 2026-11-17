from __future__ import annotations

from worker.domain.video import candidates
from worker.domain.video.ytdlp import Found
from worker.settings import Settings


def found(ref: str, title: str, duration: float | None) -> Found:
    return Found(ref, title, duration, f"https://www.youtube.com/watch?v={ref}")


def test_a_cover_and_a_wrong_length_are_dropped(
    settings: Settings,
) -> None:
    results = [
        found("cover", "Central Cee - Doja (cover)", 95.0),
        found("long", "Central Cee - Doja (Official Video)", 600.0),
        found("lyrics", "Central Cee Doja lyrics", 94.0),
        found("official", "Central Cee - Doja (Official Music Video)", 96.0),
        found("none", "Central Cee - Doja", None),
    ]
    picked = candidates.pick(results, "Central Cee", "Doja", 96.0, settings.video)
    assert sorted(item.ref for item in picked) == ["lyrics", "official"]


def test_a_negative_word_in_our_own_title_is_allowed(settings: Settings) -> None:
    results = [found("live", "Artist - Song (Live)", 200.0)]
    assert candidates.pick(results, "Artist", "Song (Live)", 200.0, settings.video)


def test_the_candidate_count_is_capped(settings: Settings) -> None:
    results = [found(str(n), f"Artist - Song {n}", 200.0) for n in range(9)]
    picked = candidates.pick(results, "Artist", "Song", 200.0, settings.video)
    assert len(picked) == settings.video.candidates_per_search
