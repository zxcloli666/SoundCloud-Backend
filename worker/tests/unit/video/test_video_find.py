from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import pytest

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, Status
from worker.domain.video_find import Candidate, InputsGone, VideoFindLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings


class Harness:
    def __init__(
        self,
        server: FakeAudioServer,
        session: aiohttp.ClientSession,
        settings: Settings,
        work_dir: Path,
    ) -> None:
        self.server = server
        self.folder = work_dir
        counters = Counters()
        source = AudioSource(
            session,
            timeout_s=settings.video.download_timeout_s,
            idle_s=settings.audio.download_idle_s,
            max_bytes=1 << 20,
            counters=counters,
        )
        self.lane = VideoFindLane(
            FakeEngines(),
            source,
            source,
            Workspace(work_dir, counters),
            settings.audio,
            settings.video,
            524_288,
            counters,
        )

    async def download(self, path: str) -> Path:
        candidate = Candidate(index=3, kind="video", url=self.server.url(path))
        return await self.lane._download(candidate, self.folder, Deadline.after(30))


@pytest.fixture
async def harness(settings: Settings, work_dir: Path) -> AsyncIterator[Harness]:
    async with FakeAudioServer() as server, aiohttp.ClientSession() as session:
        server.body = b"mp4"
        yield Harness(server, session, settings, work_dir)


async def test_a_candidate_file_lands_next_to_its_index(harness: Harness) -> None:
    path = await harness.download("/ok.wav")

    assert path.name == "c3.mp4"
    assert path.read_bytes() == b"mp4"


@pytest.mark.parametrize("missing", ["/missing", "/gone", "/forbidden"])
async def test_a_candidate_file_that_is_gone_sends_the_task_back_to_jobs(
    harness: Harness, missing: str
) -> None:
    with pytest.raises(InputsGone) as gone:
        await harness.download(missing)

    outcome = gone.value.outcome()
    assert (outcome.status, outcome.reason) == (Status.MISSING, Reason.INPUTS_GONE)


async def test_a_candidate_server_error_is_not_a_gone_file(harness: Harness) -> None:
    with pytest.raises(Exception) as failed:
        await harness.download("/error")

    assert not isinstance(failed.value, InputsGone)


async def test_a_candidate_over_the_size_cap_is_skipped_not_gone(harness: Harness) -> None:
    harness.server.body = b"x" * (2 << 20)

    with pytest.raises(PermanentFailure) as failed:
        await harness.download("/ok.wav")

    assert not isinstance(failed.value, InputsGone)
    assert failed.value.reason is Reason.AUDIO_TOO_LARGE
