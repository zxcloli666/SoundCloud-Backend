from __future__ import annotations

import asyncio
from collections.abc import Mapping

from worker.domain import embedding
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.fields import required_int, required_text
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.ports import Engines
from worker.domain.video_gen import files, media
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import VideoGenSection

LANE = "video-keyframe"


class VideoKeyframeLane:
    def __init__(
        self,
        engines: Engines,
        source: AudioSource,
        workspace: Workspace,
        settings: VideoGenSection,
        part_bytes: int,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._source = source
        self._workspace = workspace
        self._settings = settings
        self._part_bytes = part_bytes
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._draw(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _draw(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        track = required_text(request, "sc_track_id")
        width = required_int(request, "width")
        height = required_int(request, "height")
        url = required_text(request, "cover_url")
        with self._workspace.task(f"keyframe-{track}") as folder:
            cover = await files.fetch(self._source, url, folder / "cover", "cover", deadline)
            image = await asyncio.to_thread(media.read_image, cover, "cover")
        args = {
            "prompt": required_text(request, "prompt"),
            "negative_prompt": str(request.get("negative_prompt", "")),
            "width": width,
            "height": height,
            "seed": required_int(request, "seed"),
            "steps": self._settings.keyframe.steps,
            "true_cfg_scale": self._settings.keyframe.true_cfg_scale,
        }
        frame = await self._engines.edit_image(image, args, deadline)
        if frame.shape != (height, width, 3):
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"frame shape {frame.shape}")
        encoded = await asyncio.to_thread(media.png, frame)
        return files.delivered([("frame.png", encoded)], self._part_bytes)
