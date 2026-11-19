from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import aiohttp
import pytest

from tests.fakes.http_audio import FakeAudioServer
from tests.unit.video_gen.harness import Harness
from worker.settings import Settings


@pytest.fixture
async def harness(settings: Settings, work_dir: Path) -> AsyncIterator[Harness]:
    async with FakeAudioServer() as server, aiohttp.ClientSession() as session:
        yield Harness(server, session, settings, work_dir)
