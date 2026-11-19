from __future__ import annotations

import asyncio
import hashlib
import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from worker.domain import embedding
from worker.domain.audio import decode
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.fields import required_int, required_list, required_mapping, required_text
from worker.domain.outcome import Outcome, PermanentFailure, Reason, TransientFailure
from worker.domain.ports import Engines, Float32Array
from worker.domain.video import features, match, publish
from worker.domain.video.classify import Verdict, classify
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import AudioSection, VideoSection

LANE = "video-find"
CANVAS = "canvas"
SKIPPED = (PermanentFailure,)
MS = 1000.0

log = logging.getLogger(__name__)


class InputsGone(PermanentFailure):
    def __init__(self, detail: str) -> None:
        super().__init__(Reason.INPUTS_GONE, detail)


@dataclass(frozen=True)
class Candidate:
    index: int
    kind: str
    url: str


@dataclass(frozen=True)
class Job:
    track_id: str
    attempt: int
    audio_url: str
    candidates: Sequence[Candidate]


@dataclass(frozen=True)
class Track:
    mono: Float32Array
    raw: np.ndarray
    duration_s: float


@dataclass(frozen=True)
class Evaluated:
    candidate: Candidate
    path: Path
    offset_ms: int
    proof: Mapping[str, float]
    measured: features.Features
    verdict: Verdict

    @property
    def rank(self) -> int:
        if self.verdict.kind == "static":
            return 0 if not self.verdict.uncertain else 1
        base = 4 if self.verdict.kind == "clip" else 3
        return base - 2 if self.verdict.uncertain else base


class VideoFindLane:
    def __init__(
        self,
        engines: Engines,
        track_source: AudioSource,
        candidate_source: AudioSource,
        workspace: Workspace,
        audio: AudioSection,
        video: VideoSection,
        part_bytes: int,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._track_source = track_source
        self._candidate_source = candidate_source
        self._workspace = workspace
        self._audio = audio
        self._video = video
        self._part_bytes = part_bytes
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._find(parse(request), deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _find(self, job: Job, deadline: Deadline) -> Outcome:
        with self._workspace.task(f"video-{job.track_id}-{job.attempt}") as folder:
            track = await self._track(job, folder, deadline)
            if isinstance(track, Outcome):
                return track
            best, verified = await self._search(job, track, folder, deadline)
            if best is None or (best.verdict.kind == "static" and not best.verdict.uncertain):
                reason = Reason.STATIC_VIDEO if verified else Reason.NO_VIDEO_FOUND
                return Outcome.of(reason, best.verdict.rule if best else None)
            return await self._deliver(best, folder, deadline)

    async def _track(self, job: Job, folder: Path, deadline: Deadline) -> Track | Outcome:
        path = folder / "track"
        await self._track_source.fetch(job.audio_url, path, deadline)
        pcm = await decode.decode(
            path, max_duration_s=self._audio.max_duration_s, deadline=deadline
        )
        mono = decode.to_mono(pcm)
        loudness = decode.rms_dbfs(mono)
        if loudness < self._audio.silence_dbfs:
            return Outcome.of(Reason.SILENT_AUDIO, f"rms_dbfs={loudness:.1f}")
        signal = decode.resample(mono, pcm.sample_rate, match.FINGERPRINT_RATE)
        raw = await self._raw(signal, deadline)
        if raw is None:
            raise TransientFailure(Reason.INTERNAL_ERROR, "track fingerprint unavailable")
        return Track(signal, raw, pcm.duration_s)

    async def _raw(self, signal: Float32Array, deadline: Deadline) -> np.ndarray | None:
        text = await self._engines.fingerprint(
            match.int16_mono(signal), match.FINGERPRINT_RATE, 1, deadline
        )
        return match.parse_raw(text)

    async def _search(
        self, job: Job, track: Track, folder: Path, deadline: Deadline
    ) -> tuple[Evaluated | None, int]:
        best: Evaluated | None = None
        verified = 0
        for candidate in job.candidates:
            evaluated = await self._evaluate(candidate, track, folder, deadline)
            if evaluated is None:
                continue
            verified += 1
            if best is None or evaluated.rank > best.rank:
                drop(best)
                best = evaluated
            else:
                drop(evaluated)
            if best.rank == 4 or verified >= self._video.max_verified:
                break
        return best, verified

    async def _evaluate(
        self, candidate: Candidate, track: Track, folder: Path, deadline: Deadline
    ) -> Evaluated | None:
        try:
            path = await self._download(candidate, folder, deadline)
            checked = await self._check(candidate, path, track, deadline)
            if checked is None:
                drop_path(path)
                return None
            offset_ms, proof = checked
            info = await features.probe(path, deadline)
            measured = await features.measure(
                path, info, self._engines, self._video.analysis_s, deadline
            )
        except InputsGone:
            raise
        except SKIPPED as error:
            self._counters.inc("video_candidate_skipped_total")
            log.warning(
                "video candidate skipped",
                extra={"index": candidate.index, "error": str(error)},
            )
            return None
        verdict = classify(measured, self._video.classifier)
        log.info(
            "video candidate measured",
            extra={"index": candidate.index, "kind": verdict.kind, "rule": verdict.rule},
        )
        return Evaluated(candidate, path, offset_ms, proof, measured, verdict)

    async def _download(self, candidate: Candidate, folder: Path, deadline: Deadline) -> Path:
        path = folder / f"c{candidate.index}.mp4"
        try:
            await self._candidate_source.fetch(candidate.url, path, deadline)
        except PermanentFailure as failure:
            if failure.reason in (Reason.AUDIO_NOT_FOUND, Reason.AUDIO_FORBIDDEN):
                raise InputsGone(f"candidate={candidate.index} {failure.detail}") from failure
            raise
        return path

    async def _check(
        self, candidate: Candidate, path: Path, track: Track, deadline: Deadline
    ) -> tuple[int, Mapping[str, float]] | None:
        if candidate.kind == CANVAS:
            return 0, {}
        found = await self._verify(path, track, deadline)
        if found is None:
            return None
        if found.offset_ms < -self._video.max_pad_s * MS:
            self._counters.inc("video_candidate_skipped_total")
            return None
        proof = {
            "ber": found.ber,
            "peak": float(found.peak),
            "overlap_s": found.overlap_s,
            "chroma": found.chroma,
        }
        return found.offset_ms, proof

    async def _verify(self, path: Path, track: Track, deadline: Deadline) -> match.Match | None:
        pcm = await decode.decode(
            path, max_duration_s=self._video.max_candidate_s + 60.0, deadline=deadline
        )
        signal = decode.resample(decode.to_mono(pcm), pcm.sample_rate, match.FINGERPRINT_RATE)
        raw = await self._raw(signal, deadline)
        if raw is None:
            return None
        return await asyncio.to_thread(
            match.verify, track.raw, raw, track.mono, signal, self._video
        )

    async def _deliver(self, best: Evaluated, folder: Path, deadline: Deadline) -> Outcome:
        kind = "loop" if best.verdict.kind == "loop" else "clip"
        scores = {**best.measured.scores(), **best.proof}
        if best.verdict.uncertain:
            info = await features.probe(best.path, deadline)
            return Outcome.ok(
                kind=kind,
                uncertain=True,
                candidate=best.candidate.index,
                offset_ms=0 if kind == "loop" else max(best.offset_ms, 0),
                duration_ms=max(1, round(info.duration_s * MS)),
                loop=kind == "loop",
                scores=scores,
            )
        return await self._encoded(best, kind, folder, scores, deadline)

    async def _encoded(
        self,
        best: Evaluated,
        kind: str,
        folder: Path,
        scores: Mapping[str, float],
        deadline: Deadline,
    ) -> Outcome:
        looped = kind == "loop"
        offset_ms = 0 if looped else best.offset_ms
        target = folder / "out.mp4"
        await publish.encode(
            best.path,
            target,
            pad_s=max(-offset_ms, 0) / MS,
            settings=self._video,
            deadline=deadline,
        )
        encoded = await features.probe(target, deadline)
        blob = await asyncio.to_thread(target.read_bytes)
        parts = publish.split(blob, self._part_bytes)
        return Outcome.ok_in_parts(
            parts,
            kind=kind,
            uncertain=False,
            candidate=best.candidate.index,
            offset_ms=max(offset_ms, 0),
            duration_ms=max(1, round(encoded.duration_s * MS)),
            loop=looped,
            scores=scores,
            clip={
                "bytes": len(blob),
                "sha256": hashlib.sha256(blob).hexdigest(),
                "parts": len(parts),
            },
        )


def drop(evaluated: Evaluated | None) -> None:
    if evaluated is not None:
        drop_path(evaluated.path)


def drop_path(path: Path) -> None:
    path.unlink(missing_ok=True)


def parse(request: Mapping[str, object]) -> Job:
    return Job(
        track_id=required_text(request, "sc_track_id"),
        attempt=required_int(request, "attempt"),
        audio_url=required_text(request, "s3_url"),
        candidates=[
            candidate(required_mapping({"c": item}, "c"))
            for item in required_list(request, "candidates")
        ],
    )


def candidate(raw: Mapping[str, object]) -> Candidate:
    return Candidate(
        index=required_int(raw, "index"),
        kind=required_text(raw, "kind"),
        url=required_text(raw, "url"),
    )
