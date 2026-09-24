from __future__ import annotations

import pytest

from worker.domain.metadata.normalize import (
    find_name,
    fold,
    is_reupload_channel,
    parse_title,
    split_credits,
    version_markers,
    words,
)


def test_a_plain_artist_dash_title_is_split() -> None:
    parts = parse_title("Billie Eilish - Ocean Eyes")

    assert parts.artist == "Billie Eilish"
    assert parts.song == "Ocean Eyes"


@pytest.mark.parametrize("dash", ["-", "–", "—", "|"])
def test_every_separator_shape_splits(dash: str) -> None:
    parts = parse_title(f"The Weeknd {dash} Blinding Lights")

    assert (parts.artist, parts.song) == ("The Weeknd", "Blinding Lights")


def test_noise_brackets_are_dropped() -> None:
    parts = parse_title("Artist - Song [Official Video] (HD) (Lyrics) [FREE DL]")

    assert parts.song == "Song"
    assert parts.versions == ()


def test_version_tags_are_kept_aside() -> None:
    parts = parse_title("Post Malone - Sunflower (Slowed + Reverb)")

    assert parts.song == "Sunflower"
    assert parts.versions == ("slowed + reverb",)


def test_featured_artist_before_the_dash_is_pulled_out() -> None:
    parts = parse_title("Post Malone ft. Swae Lee - Sunflower")

    assert parts.artist == "Post Malone"
    assert parts.featured == ("Swae Lee",)
    assert parts.song == "Sunflower"


def test_featured_artists_inside_brackets_are_split() -> None:
    parts = parse_title("Artist - Song (feat. A, B & C)")

    assert parts.featured == ("A", "B", "C")
    assert parts.song == "Song"


def test_producer_and_remixer_are_recognised() -> None:
    produced = parse_title("Artist - Song (prod. Metro Boomin)")
    remixed = parse_title("Artist - Song (Skrillex Remix)")

    assert produced.producers == ("Metro Boomin",)
    assert remixed.remixers == ("Skrillex",)
    assert remixed.song == "Song"


def test_emoji_and_handles_are_stripped() -> None:
    parts = parse_title("Artist - Song 🔥🔥 @promo #trap")

    assert parts.song == "Song"


def test_a_title_without_separator_has_no_artist() -> None:
    parts = parse_title("Just A Song Name")

    assert parts.artist is None
    assert parts.song == "Just A Song Name"


def test_cyrillic_is_kept_in_the_parsed_title() -> None:
    parts = parse_title("Psychosis — ливень")

    assert (parts.artist, parts.song) == ("Psychosis", "ливень")


def test_empty_title_does_not_explode() -> None:
    parts = parse_title("")

    assert parts.artist is None
    assert parts.song == ""


def test_metadata_artist_credits_split_featured() -> None:
    credits = split_credits("Kendrick Lamar feat. SZA")

    assert credits.main == "Kendrick Lamar"
    assert credits.featured == ("SZA",)


@pytest.mark.parametrize(
    ("uploader", "channel"),
    [
        ("Nightcore Vibes", True),
        ("Chill Beats Radio", True),
        ("Trap Nation Records", True),
        ("Psychosis", False),
        ("Billie Eilish", False),
    ],
)
def test_reupload_channels_are_recognised(uploader: str, channel: bool) -> None:
    assert is_reupload_channel(uploader) is channel


@pytest.mark.parametrize(
    ("title", "markers"),
    [
        ("Song", set()),
        ("Song (Skrillex Remix)", {"remix"}),
        ("Song (Live)", {"live"}),
        ("Live Forever", set()),
        ("Song - Sped Up", {"sped_up"}),
        ("Song [Nightcore]", {"sped_up"}),
        ("Song (slowed + reverb)", {"slowed"}),
        ("Song (Acoustic Version)", {"acoustic"}),
        ("Song (Instrumental)", {"instrumental"}),
    ],
)
def test_version_markers(title: str, markers: set[str]) -> None:
    assert version_markers(title) == markers


def test_folding_removes_case_symbols_and_script() -> None:
    assert fold("  Ocean-Eyes!! ") == "ocean eyes"
    assert fold("Ливень") == "liven"
    assert words("Beyoncé") == ["beyonce"]


def test_find_name_needs_the_whole_word_sequence() -> None:
    text = "Kendrick Lamar - Luther (with SZA)"

    assert find_name(text, "SZA") == "SZA"
    assert find_name(text, "kendrick lamar") == "Kendrick Lamar"
    assert find_name(text, "KENDRICK LAMAR!!") == "Kendrick Lamar"
    assert find_name(text, "Lamar Kendrick") is None
    assert find_name(text, "Ken") is None
    assert find_name(text, "!!!") is None


def test_find_name_does_not_transliterate() -> None:
    assert find_name("Zemfira - Iskala", "Земфира") is None
    assert find_name("Земфира - Искала", "Zemfira") is None
    assert find_name("Beyoncé - Halo", "Beyonce") is None
    assert find_name("Земфира - Искала", "ИСКАЛА") == "Искала"
