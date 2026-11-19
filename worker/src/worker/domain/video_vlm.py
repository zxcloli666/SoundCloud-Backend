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
from worker.domain.ports import Engines, Uint8Array
from worker.domain.video import features, publish, sample
from worker.domain.video_find import InputsGone
from worker.domain.video_gen import files, media
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import VideoSection, VlmSettings

LANE = "video-vlm"
LABELS = ("clip", "loop", "static")
STATIC = "static"
MS = 1000.0
FRAME_QUALITY = 90

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Job:
    track_id: str
    attempt: int
    video_url: str
    duration_ms: int | None
    offset_ms: int | None
    frames_only: bool
    label: str | None
    prompt: str | None


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
            return await self._run(parse(request), deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _run(self, job: Job, deadline: Deadline) -> Outcome:
        if job.label == STATIC:
            return Outcome.ok(label=STATIC)
        with self._workspace.task(f"vlm-{job.track_id}-{job.attempt}") as folder:
            path = folder / "video.mp4"
            await self._download(job, path, deadline)
            if job.label is not None:
                return await self._encoded(job.label, path, folder, deadline)
            frames = await self._frames(job, path, deadline)
            if job.frames_only:
                return await self._delivered(frames)
            return await self._classify(job, frames, path, folder, deadline)

    async def _download(self, job: Job, path: Path, deadline: Deadline) -> None:
        try:
            await self._source.fetch(job.video_url, path, deadline)
        except PermanentFailure as failure:
            if failure.reason in (Reason.AUDIO_NOT_FOUND, Reason.AUDIO_FORBIDDEN):
                raise InputsGone(failure.detail or failure.reason.value) from failure
            raise

    async def _frames(self, job: Job, path: Path, deadline: Deadline) -> Uint8Array:
        info = await features.probe(path, deadline)
        start, span = sample.window(info.duration_s, job.offset_ms, job.duration_ms)
        return await sample.frames(
            path,
            start_s=start,
            span_s=span,
            count=self._vlm.frames,
            side=self._vlm.frame_side,
            deadline=deadline,
        )

    async def _delivered(self, frames: Uint8Array) -> Outcome:
        named = await asyncio.to_thread(
            lambda: [
                (f"frame{index}.jpg", media.jpeg(frame, FRAME_QUALITY))
                for index, frame in enumerate(frames)
            ]
        )
        log.info("video frames cut", extra={"frames": len(named)})
        return files.delivered(named, self._part_bytes)

    async def _classify(
        self, job: Job, frames: Uint8Array, path: Path, folder: Path, deadline: Deadline
    ) -> Outcome:
        if job.prompt is None:
            raise PermanentFailure(Reason.INVALID_REQUEST, "prompt must be a string")
        verdict = await self._engines.classify_frames(frames, job.prompt, LABELS, deadline)
        log.info(
            "video classified",
            extra={"label": verdict.label, "frames": int(frames.shape[0])},
        )
        scores = {name: round(value, 4) for name, value in verdict.scores.items()}
        if verdict.label == STATIC:
            return Outcome.ok(label=verdict.label, model=self._model, scores=scores)
        return await self._encoded(
            verdict.label, path, folder, deadline, model=self._model, scores=scores
        )

    async def _encoded(
        self,
        label: str,
        path: Path,
        folder: Path,
        deadline: Deadline,
        **verdict: object,
    ) -> Outcome:
        target = folder / "out.mp4"
        await publish.encode(path, target, pad_s=0.0, settings=self._video, deadline=deadline)
        encoded = await features.probe(target, deadline)
        blob = await asyncio.to_thread(target.read_bytes)
        parts = publish.split(blob, self._part_bytes)
        return Outcome.ok_in_parts(
            parts,
            label=label,
            **verdict,
            duration_ms=max(1, round(encoded.duration_s * MS)),
            clip={
                "bytes": len(blob),
                "sha256": hashlib.sha256(blob).hexdigest(),
                "parts": len(parts),
            },
        )


def parse(request: Mapping[str, object]) -> Job:
    label = request.get("label")
    if label is not None and label not in LABELS:
        raise PermanentFailure(Reason.INVALID_REQUEST, f"label must be one of {list(LABELS)}")
    prompt = request.get("prompt")
    return Job(
        track_id=required_text(request, "sc_track_id"),
        attempt=required_int(request, "attempt"),
        video_url=required_text(request, "video_url"),
        duration_ms=optional_int(request, "duration_ms"),
        offset_ms=optional_int(request, "offset_ms"),
        frames_only=request.get("frames") is True,
        label=label if isinstance(label, str) else None,
        prompt=prompt if isinstance(prompt, str) and prompt else None,
    )


def optional_int(request: Mapping[str, object], key: str) -> int | None:
    if request.get(key) is None:
        return None
    return required_int(request, key)
