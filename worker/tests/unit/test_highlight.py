from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import numpy as np
import pytest

from tests.fakes.http_audio import FakeAudioServer
from tests.unit.test_audio_decode import wav_bytes
from worker.domain.audio import moment
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.highlight import HighlightLane
from worker.domain.outcome import Outcome, Reason, Status
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

RATE = 22_050
QUIET = 0.05
MEDIUM = 0.15
LOUD = 0.4
INTRO = (220.0, 261.63)
VERSE = (293.66, 369.99)
CHORUS = (329.63, 415.30, 493.88)
BRIDGE = (369.99, 466.16, 277.18)
OUTRO = (196.0, 246.94)


def section(seconds: float, notes: tuple[float, ...], amplitude: float, seed: int) -> np.ndarray:
    t = np.arange(int(seconds * RATE)) / RATE
    tone = sum(np.sin(2 * np.pi * note * t) + 0.3 * np.sin(4 * np.pi * note * t) for note in notes)
    pulse = 0.75 + 0.25 * np.sin(2 * np.pi * 2 * t)
    noise = np.random.default_rng(seed).normal(0.0, 0.01, t.shape[0])
    return (amplitude * pulse * tone / len(notes) + noise).astype(np.float32)


def song(*parts: tuple[float, tuple[float, ...], float]) -> np.ndarray:
    return np.concatenate(
        [section(seconds, notes, level, seed) for seed, (seconds, notes, level) in enumerate(parts)]
    )


def starts_in(found: moment.Moment, *sections_s: float) -> bool:
    return any(abs(found.start_ms / 1000 - start) <= 2.5 for start in sections_s)


def test_a_repeated_loud_chorus_beats_an_equally_loud_bridge() -> None:
    track = song(
        (20, INTRO, QUIET),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, BRIDGE, LOUD),
        (30, CHORUS, LOUD),
        (20, OUTRO, QUIET),
    )

    found = moment.find_moment(track, RATE)

    assert starts_in(found, 50, 140)
    assert found.length_ms == 30_000
    assert 0.0 <= found.score <= 1.0


def test_an_intro_never_wins_when_a_chorus_exists() -> None:
    track = song(
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (20, OUTRO, QUIET),
    )

    found = moment.find_moment(track, RATE)

    assert found.start_ms >= 0.08 * 170 * 1000
    assert starts_in(found, 60, 120)


def test_a_track_shorter_than_the_window_plays_whole() -> None:
    found = moment.find_moment(section(24, CHORUS, LOUD, 1), RATE)

    assert found == moment.Moment(0, 24_000, moment.SHORT_TRACK_SCORE)


def test_the_window_never_runs_past_the_end() -> None:
    found = moment.find_moment(song((20, VERSE, MEDIUM), (20, CHORUS, LOUD)), RATE)

    assert found.start_ms + found.length_ms <= 40_000
    assert found.length_ms == 30_000


def test_the_start_lands_on_the_section_edge_off_the_step_grid() -> None:
    track = song(
        (21.3, INTRO, QUIET),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (20, OUTRO, QUIET),
    )

    found = moment.find_moment(track, RATE)

    assert any(abs(found.start_ms / 1000 - start) <= 0.6 for start in (51.3, 111.3))


def test_a_slightly_louder_bridge_still_loses_to_the_chorus() -> None:
    track = song(
        (20, INTRO, QUIET),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, BRIDGE, LOUD * 1.15),
        (30, CHORUS, LOUD),
        (20, OUTRO, QUIET),
    )

    found = moment.find_moment(track, RATE)

    assert starts_in(found, 50, 140)


def test_a_loud_unique_intro_riff_never_wins() -> None:
    track = song(
        (25, INTRO, LOUD * 1.2),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (30, VERSE, MEDIUM),
        (30, CHORUS, LOUD),
        (20, OUTRO, QUIET),
    )

    found = moment.find_moment(track, RATE)

    assert starts_in(found, 55, 115)


def test_leading_silence_never_wins() -> None:
    track = np.concatenate(
        [
            np.zeros(RATE * 15, dtype=np.float32),
            song((30, VERSE, MEDIUM), (30, CHORUS, LOUD), (30, VERSE, MEDIUM), (30, CHORUS, LOUD)),
        ]
    )

    found = moment.find_moment(track, RATE)

    assert starts_in(found, 45, 105)


def test_a_track_just_past_the_window_keeps_a_full_window_inside() -> None:
    found = moment.find_moment(section(31, CHORUS, LOUD, 2), RATE)

    assert found.length_ms == 30_000
    assert 0 <= found.start_ms <= 1_000


def test_a_track_exactly_one_window_long_plays_whole() -> None:
    found = moment.find_moment(section(30, CHORUS, LOUD, 3), RATE)

    assert found == moment.Moment(0, 30_000, moment.SHORT_TRACK_SCORE)


def test_the_same_audio_gives_the_same_moment() -> None:
    track = song((20, INTRO, QUIET), (30, VERSE, MEDIUM), (30, CHORUS, LOUD), (30, VERSE, MEDIUM))

    assert moment.find_moment(track, RATE) == moment.find_moment(track.copy(), RATE)


class Harness:
    def __init__(
        self,
        server: FakeAudioServer,
        session: aiohttp.ClientSession,
        settings: Settings,
        work_dir: Path,
    ) -> None:
        self.server = server
        counters = Counters()
        audio = settings.audio
        source = AudioSource(
            session,
            timeout_s=audio.download_timeout_s,
            idle_s=audio.download_idle_s,
            max_bytes=audio.max_download_mib << 20,
            counters=counters,
        )
        self.lane = HighlightLane(source, Workspace(work_dir, counters), audio, counters)

    async def run(self, body: bytes, path: str = "/ok.wav") -> Outcome:
        self.server.body = body
        request = {"sc_track_id": "98765", "s3_url": self.server.url(path), "attempt": 1}
        return await self.lane.process({**request, "version": 1}, Deadline.after(60))


@pytest.fixture
async def harness(settings: Settings, work_dir: Path) -> AsyncIterator[Harness]:
    async with FakeAudioServer() as server, aiohttp.ClientSession() as session:
        yield Harness(server, session, settings, work_dir)


async def test_the_lane_publishes_the_moment_with_its_algorithm_version(harness: Harness) -> None:
    track = song((30, VERSE, MEDIUM), (30, CHORUS, LOUD), (30, VERSE, MEDIUM), (30, CHORUS, LOUD))

    outcome = await harness.run(wav_bytes(np.stack([track, track], axis=1), RATE))

    assert outcome.status is Status.OK
    assert outcome.fields["version"] == moment.VERSION
    assert outcome.fields["length_ms"] == 30_000
    assert 0 <= int(str(outcome.fields["start_ms"])) <= 90_000


async def test_silence_has_no_moment(harness: Harness) -> None:
    outcome = await harness.run(wav_bytes(np.zeros((RATE * 40, 2), dtype=np.float32), RATE))

    assert (outcome.status, outcome.reason) == (Status.EMPTY, Reason.SILENT_AUDIO)


async def test_a_missing_object_is_missing(harness: Harness) -> None:
    outcome = await harness.run(b"", path="/missing")

    assert outcome.status is Status.MISSING
