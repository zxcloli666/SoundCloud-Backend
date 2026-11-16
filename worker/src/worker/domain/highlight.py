from __future__ import annotations

import asyncio
from collections.abc import Mapping

from worker.domain import embedding
from worker.domain.audio import decode, moment
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Reason
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import AudioSection

LANE = "highlight"


class HighlightLane:
    def __init__(
        self,
        source: AudioSource,
        workspace: Workspace,
        settings: AudioSection,
        counters: Counters,
    ) -> None:
        self._source = source
        self._workspace = workspace
        self._settings = settings
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._highlight(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _highlight(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        track_id = embedding.text_field(request, "sc_track_id")
        url = embedding.text_field(request, "s3_url")
        with self._workspace.task(f"highlight-{track_id}") as folder:
            audio_path = folder / "audio"
            await self._source.fetch(url, audio_path, deadline)
            pcm = await decode.decode(
                audio_path, max_duration_s=self._settings.max_duration_s, deadline=deadline
            )
        mono = decode.to_mono(pcm)
        loudness = decode.rms_dbfs(mono)
        if loudness < self._settings.silence_dbfs:
            return Outcome.of(Reason.SILENT_AUDIO, f"rms_dbfs={loudness:.1f}")
        deadline.check("highlight")
        found = await asyncio.to_thread(
            moment.find_moment_single_thread, mono, pcm.sample_rate, moment.hints_of(request)
        )
        return Outcome.ok(
            version=moment.VERSION,
            start_ms=found.start_ms,
            length_ms=found.length_ms,
            score=round(found.score, 4),
        )
