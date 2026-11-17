from __future__ import annotations

import stat
from pathlib import Path

import orjson

from worker.domain.outcome import Reason, TransientFailure
from worker.domain.video import ytdlp
from worker.observability.counters import Counters
from worker.settings import Settings


def test_cookies_live_in_a_private_file_and_go_away(tmp_path: Path) -> None:
    with ytdlp.cookie_file(tmp_path, "# Netscape HTTP Cookie File\n") as path:
        assert path is not None
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert path.read_text().startswith("# Netscape")
    assert not path.exists()


def test_no_cookies_means_no_file(tmp_path: Path) -> None:
    with ytdlp.cookie_file(tmp_path, None) as path:
        assert path is None
    assert list(tmp_path.iterdir()) == []


def test_the_file_is_removed_when_the_body_raises(tmp_path: Path) -> None:
    try:
        with ytdlp.cookie_file(tmp_path, "x") as path:
            raise RuntimeError
    except RuntimeError:
        pass
    assert path is not None
    assert not path.exists()


def test_search_json_becomes_candidates() -> None:
    body = orjson.dumps(
        {"entries": [{"id": "abc", "title": "A - B", "duration": 190}, {"title": "no id"}]}
    )
    [found] = ytdlp.parse_search(body)
    assert (found.ref, found.duration_s, found.url) == (
        "abc",
        190.0,
        "https://www.youtube.com/watch?v=abc",
    )


def failure(settings: Settings, stderr: str) -> Exception:
    return ytdlp.YtDlp(settings.video, Counters())._failure("download", stderr)


def test_a_bot_check_is_a_rate_limit(settings: Settings) -> None:
    error = failure(settings, "ERROR: Sign in to confirm you're not a bot")
    assert isinstance(error, TransientFailure)
    assert error.reason is Reason.SOURCE_RATE_LIMITED


def test_dead_cookies_are_an_auth_failure(settings: Settings) -> None:
    error = failure(settings, "The provided YouTube account cookies are no longer valid")
    assert isinstance(error, TransientFailure)
    assert error.reason is Reason.SOURCE_AUTH_FAILED


def test_a_private_video_only_skips_the_candidate(settings: Settings) -> None:
    assert isinstance(failure(settings, "ERROR: Video unavailable"), ytdlp.CandidateUnavailable)
