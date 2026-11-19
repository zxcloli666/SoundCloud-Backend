from __future__ import annotations

import asyncio
import hashlib
import logging
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from worker.domain import embedding
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.fields import required_int, required_text
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.ports import Engines
from worker.domain.video import features, publish, sample
from worker.domain.video_find import InputsGone
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import VideoSection, VlmSettings

LANE = "video-vlm"
LABELS = ("clip", "loop", "static")
CLIP = "clip"
MS = 1000.0
PROMPT = (
    "You see frames from a music video, sampled evenly in time and shown in order.\n"
    "Classify the visual track into one class:\n"
    "static = the same single picture the whole time. It may have a slow zoom or pan, a waveform "
    "or spectrum drawn over the still picture, or subtle overlays on the same picture "
    "(noise, grain, flicker, particles, color shifts).\n"
    "loop = a short segment of real motion that repeats again and again; the same frames come "
    "back in a cycle.\n"
    "clip = real footage or animation that changes over time, with different shots, cuts or "
    "scenes, and no short repetition.\n"
    "Answer with one word: clip, loop or static."
)

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Job:
    track_id: str
    attempt: int
    video_url: str
    duration_ms: int | None
    offset_ms: int | None


class VideoVlmLane:
    def __init__(
        self,
        engines: Engines,
        source: AudioSource,
        workspace: Workspace,
        video: VideoSection,
        vlm: VlmSettings,
        model: str,
        part_bytes: int,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._source = source
        self._workspace = workspace
        self._video = video
        self._vlm = vlm
        self._model = model
        self._part_bytes = part_bytes
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._classify(parse(request), deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _classify(self, job: Job, deadline: Deadline) -> Outcome:
        with self._workspace.task(f"vlm-{job.track_id}-{job.attempt}") as folder:
            path = folder / "video.mp4"
            await self._download(job, path, deadline)
            info = await features.probe(path, deadline)
            start, span = sample.window(info.duration_s, job.offset_ms, job.duration_ms)
            frames = await sample.frames(
                path,
                start_s=start,
                span_s=span,
                count=self._vlm.frames,
                side=self._vlm.frame_side,
                deadline=deadline,
            )
            verdict = await self._engines.classify_frames(frames, PROMPT, LABELS, deadline)
            log.info(
                "video classified",
                extra={"label": verdict.label, "frames": int(frames.shape[0])},
            )
            scores = {name: round(value, 4) for name, value in verdict.scores.items()}
            if verdict.label != CLIP:
                return Outcome.ok(label=verdict.label, model=self._model, scores=scores)
            return await self._clip(path, folder, scores, deadline)

    async def _download(self, job: Job, path: Path, deadline: Deadline) -> None:
        try:
            await self._source.fetch(job.video_url, path, deadline)
        except PermanentFailure as failure:
            if failure.reason in (Reason.AUDIO_NOT_FOUND, Reason.AUDIO_FORBIDDEN):
                raise InputsGone(failure.detail or failure.reason.value) from failure
            raise

    async def _clip(
        self, path: Path, folder: Path, scores: Mapping[str, float], deadline: Deadline
    ) -> Outcome:
        target = folder / "out.mp4"
        await publish.encode(path, target, pad_s=0.0, settings=self._video, deadline=deadline)
        encoded = await features.probe(target, deadline)
        blob = await asyncio.to_thread(target.read_bytes)
        parts = publish.split(blob, self._part_bytes)
        return Outcome.ok_in_parts(
            parts,
            label=CLIP,
            model=self._model,
            scores=scores,
            duration_ms=max(1, round(encoded.duration_s * MS)),
            clip={
                "bytes": len(blob),
                "sha256": hashlib.sha256(blob).hexdigest(),
                "parts": len(parts),
            },
        )


def parse(request: Mapping[str, object]) -> Job:
    return Job(
        track_id=required_text(request, "sc_track_id"),
        attempt=required_int(request, "attempt"),
        video_url=required_text(request, "video_url"),
        duration_ms=optional_int(request, "duration_ms"),
        offset_ms=optional_int(request, "offset_ms"),
    )


def optional_int(request: Mapping[str, object], key: str) -> int | None:
    if request.get(key) is None:
        return None
    return required_int(request, key)
