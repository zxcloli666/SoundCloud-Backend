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

LANE = "video-shot"


class VideoShotLane:
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
            return await self._animate(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _animate(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        track = required_text(request, "sc_track_id")
        width = required_int(request, "width")
        height = required_int(request, "height")
        url = required_text(request, "frame_url")
        with self._workspace.task(f"shot-{track}") as folder:
            path = await files.fetch(self._source, url, folder / "frame", "frame", deadline)
            image = await asyncio.to_thread(media.read_image, path, "frame")
            frames = await self._engines.animate_image(image, self._args(request), deadline)
            if frames.ndim != 4 or frames.shape[1:] != (height, width, 3):
                raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"frames shape {frames.shape}")
            target = folder / "shot.mp4"
            fps = required_int(request, "fps")
            await media.frames_to_mp4(frames, fps, target, self._settings, deadline)
            shot = await asyncio.to_thread(target.read_bytes)
        picks = media.still_indices(frames.shape[0], required_int(request, "stills"))
        stills = await asyncio.to_thread(lambda: [media.jpeg(frames[index]) for index in picks])
        named = [("shot.mp4", shot)]
        named += [(f"still{number}.jpg", still) for number, still in enumerate(stills)]
        return files.delivered(named, self._part_bytes)

    def _args(self, request: Mapping[str, object]) -> dict[str, object]:
        shot = self._settings.shot
        return {
            "prompt": required_text(request, "prompt"),
            "negative_prompt": str(request.get("negative_prompt", "")),
            "width": required_int(request, "width"),
            "height": required_int(request, "height"),
            "frames": required_int(request, "frames"),
            "seed": required_int(request, "seed"),
            "steps": shot.steps,
            "guidance_scale": shot.guidance_scale,
            "guidance_scale_2": shot.guidance_scale_2,
        }
