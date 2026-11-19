from __future__ import annotations

import asyncio
from collections.abc import Mapping
from pathlib import Path

from worker.domain import embedding
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.fields import required_int, required_list, required_mapping, required_text
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.video_gen import files, media
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import VideoGenSection

LANE = "video-assemble"


class VideoAssembleLane:
    def __init__(
        self,
        audio_source: AudioSource,
        shot_source: AudioSource,
        workspace: Workspace,
        settings: VideoGenSection,
        part_bytes: int,
        counters: Counters,
    ) -> None:
        self._audio_source = audio_source
        self._shot_source = shot_source
        self._workspace = workspace
        self._settings = settings
        self._part_bytes = part_bytes
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._assemble(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _assemble(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        track = required_text(request, "sc_track_id")
        shots = [required_mapping({"s": item}, "s") for item in required_list(request, "shots")]
        if not shots:
            raise PermanentFailure(Reason.INVALID_REQUEST, "no shots")
        audio_url = required_text(request, "audio_url")
        with self._workspace.task(f"assemble-{track}") as folder:
            cuts = [
                await self._cut(shot, index, folder, deadline) for index, shot in enumerate(shots)
            ]
            audio = await files.fetch(
                self._audio_source, audio_url, folder / "audio", "audio", deadline
            )
            target = folder / "video.mp4"
            await media.assemble(
                cuts,
                audio,
                required_int(request, "audio_offset_ms"),
                (required_int(request, "width"), required_int(request, "height")),
                required_int(request, "fps"),
                target,
                self._settings,
                deadline,
            )
            video = await asyncio.to_thread(target.read_bytes)
        return files.delivered([("video.mp4", video)], self._part_bytes)

    async def _cut(
        self, shot: Mapping[str, object], index: int, folder: Path, deadline: Deadline
    ) -> media.Cut:
        url = required_text(shot, "url")
        path = await files.fetch(
            self._shot_source, url, folder / f"shot{index}.mp4", f"shot={index}", deadline
        )
        trim = required_int(shot, "trim_start_ms")
        return media.Cut(path, trim, required_int(shot, "duration_ms"))
