from __future__ import annotations

import asyncio
import logging
import random
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
from worker.domain.video import candidates, features, match, publish
from worker.domain.video.classify import Verdict, classify
from worker.domain.video.ytdlp import CandidateUnavailable, Found, YtDlp, cookie_file
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import AudioSection, VideoSection

LANE = "video-find"
YOUTUBE = "youtube"
SEARCH = "search"
DIRECT_FORMAT = "best[height<=720]/best"
SKIPPED = (CandidateUnavailable, PermanentFailure)
MS = 1000.0

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Candidate:
    source: str
    url: str
    ref: str | None
    trusted: bool


@dataclass(frozen=True)
class Job:
    track_id: str
    attempt: int
    audio_url: str
    upload_url: str
    artist: str
    title: str
    duration_s: float | None
    candidates: Sequence[Mapping[str, object]]
    cookies: str | None


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
        ytdlp: YtDlp,
        uploader: publish.Uploader,
        workspace: Workspace,
        audio: AudioSection,
        video: VideoSection,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._track_source = track_source
        self._ytdlp = ytdlp
        self._uploader = uploader
        self._workspace = workspace
        self._audio = audio
        self._video = video
        self._counters = counters
        self._paced = False

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._find(parse(request), deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _find(self, job: Job, deadline: Deadline) -> Outcome:
        self._paced = False
        with self._workspace.task(f"video-{job.track_id}-{job.attempt}") as folder:
            track = await self._track(job, folder, deadline)
            if isinstance(track, Outcome):
                return track
            with cookie_file(folder, job.cookies) as cookies:
                best, verified = await self._search(job, track, folder, cookies, deadline)
            if best is None or (best.verdict.kind == "static" and not best.verdict.uncertain):
                reason = Reason.STATIC_VIDEO if verified else Reason.NO_VIDEO_FOUND
                return Outcome.of(reason, best.verdict.rule if best else None)
            return await self._deliver(job, best, folder, deadline)

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
        self,
        job: Job,
        track: Track,
        folder: Path,
        cookies: Path | None,
        deadline: Deadline,
    ) -> tuple[Evaluated | None, int]:
        wanted = job.duration_s or track.duration_s
        best: Evaluated | None = None
        verified = 0
        index = 0
        for candidate in await self._resolve(job, wanted, cookies, deadline):
            index += 1
            evaluated = await self._evaluate(candidate, index, track, folder, cookies, deadline)
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

    async def _resolve(
        self, job: Job, wanted_s: float, cookies: Path | None, deadline: Deadline
    ) -> list[Candidate]:
        resolved: list[Candidate] = []
        for raw in job.candidates:
            source = required_text(raw, "source")
            value = required_text(raw, "value")
            ref = raw.get("source_ref")
            ref = ref if isinstance(ref, str) else None
            if required_text(raw, "kind") == SEARCH:
                if source == YOUTUBE:
                    resolved += await self._searched(job, value, wanted_s, cookies, deadline)
                continue
            trusted = raw.get("trusted") is True
            resolved.append(Candidate(source, value, ref, trusted))
        return resolved

    async def _searched(
        self, job: Job, query: str, wanted_s: float, cookies: Path | None, deadline: Deadline
    ) -> list[Candidate]:
        await self._pause(deadline)
        try:
            found = await self._ytdlp.search(query, cookies, deadline)
        except CandidateUnavailable as error:
            self._counters.inc("video_search_failures_total")
            log.warning("video search failed", extra={"error": str(error)})
            return []
        chosen: list[Found] = candidates.pick(found, job.artist, job.title, wanted_s, self._video)
        return [Candidate(YOUTUBE, item.url, item.ref, False) for item in chosen]

    async def _evaluate(
        self,
        candidate: Candidate,
        index: int,
        track: Track,
        folder: Path,
        cookies: Path | None,
        deadline: Deadline,
    ) -> Evaluated | None:
        try:
            checked = await self._fetch(candidate, index, track, folder, cookies, deadline)
            if checked is None:
                return None
            path, offset_ms, proof = checked
            info = await features.probe(path, deadline)
            measured = await features.measure(
                path, info, self._engines, self._video.analysis_s, deadline
            )
        except SKIPPED as error:
            self._counters.inc("video_candidate_skipped_total", source=candidate.source)
            log.warning(
                "video candidate skipped",
                extra={"source": candidate.source, "ref": candidate.ref, "error": str(error)},
            )
            return None
        verdict = classify(measured, self._video.classifier)
        log.info(
            "video candidate measured",
            extra={"ref": candidate.ref, "kind": verdict.kind, "rule": verdict.rule},
        )
        return Evaluated(candidate, path, offset_ms, proof, measured, verdict)

    async def _fetch(
        self,
        candidate: Candidate,
        index: int,
        track: Track,
        folder: Path,
        cookies: Path | None,
        deadline: Deadline,
    ) -> tuple[Path, int, Mapping[str, float]] | None:
        youtube = candidate.source == YOUTUBE
        if candidate.trusted:
            return await self._download(candidate, folder / f"c{index}v", cookies, deadline), 0, {}
        if youtube:
            audio = await self._download(
                candidate, folder / f"c{index}a", cookies, deadline, self._video.audio_format
            )
            found = await self._verify(audio, track, deadline)
            if found is None:
                return None
            video = await self._download(
                candidate, folder / f"c{index}v", cookies, deadline, self._video.video_format
            )
        else:
            video = await self._download(candidate, folder / f"c{index}v", cookies, deadline)
            found = await self._verify(video, track, deadline)
            if found is None:
                return None
        if found.offset_ms < -self._video.max_pad_s * MS:
            self._counters.inc("video_candidate_skipped_total", source=candidate.source)
            return None
        proof = {
            "ber": found.ber,
            "peak": float(found.peak),
            "overlap_s": found.overlap_s,
            "chroma": found.chroma,
        }
        return video, found.offset_ms, proof

    async def _download(
        self,
        candidate: Candidate,
        stem: Path,
        cookies: Path | None,
        deadline: Deadline,
        selector: str = DIRECT_FORMAT,
    ) -> Path:
        if candidate.source == YOUTUBE:
            await self._pause(deadline)
        return await self._ytdlp.download(
            candidate.url,
            stem,
            selector,
            cookies if candidate.source == YOUTUBE else None,
            deadline,
        )

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

    async def _pause(self, deadline: Deadline) -> None:
        if self._paced:
            pause = random.uniform(self._video.youtube_pause_min_s, self._video.youtube_pause_max_s)
            await asyncio.sleep(min(pause, deadline.remaining()))
        self._paced = True

    async def _deliver(
        self, job: Job, best: Evaluated, folder: Path, deadline: Deadline
    ) -> Outcome:
        kind = "loop" if best.verdict.kind == "loop" else "clip"
        offset_ms = 0 if kind == "loop" else max(best.offset_ms, 0)
        pad_s = max(-best.offset_ms, 0) / MS if kind == "clip" else 0.0
        target = folder / "out.mp4"
        await publish.encode(
            best.path, target, pad_s=pad_s, settings=self._video, deadline=deadline
        )
        encoded = await features.probe(target, deadline)
        key = await self._uploader.put(target, job.upload_url, deadline)
        return Outcome.ok(
            kind=kind,
            uncertain=best.verdict.uncertain,
            s3_key=key,
            offset_ms=offset_ms,
            duration_ms=max(1, round(encoded.duration_s * MS)),
            loop=kind == "loop",
            source=best.candidate.source,
            source_ref=best.candidate.ref,
            scores={**best.measured.scores(), **best.proof},
        )


def drop(evaluated: Evaluated | None) -> None:
    if evaluated is not None:
        evaluated.path.unlink(missing_ok=True)


def parse(request: Mapping[str, object]) -> Job:
    duration = request.get("duration_ms")
    cookies = request.get("youtube_cookies")
    return Job(
        track_id=required_text(request, "sc_track_id"),
        attempt=required_int(request, "attempt"),
        audio_url=required_text(request, "s3_url"),
        upload_url=required_text(request, "upload_url"),
        artist=required_text(request, "artist"),
        title=required_text(request, "title"),
        duration_s=duration / MS if isinstance(duration, int) and duration > 0 else None,
        candidates=[
            required_mapping({"c": item}, "c") for item in required_list(request, "candidates")
        ],
        cookies=cookies if isinstance(cookies, str) and cookies else None,
    )
