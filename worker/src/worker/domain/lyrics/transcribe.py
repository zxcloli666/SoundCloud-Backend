from __future__ import annotations

import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, replace

import numpy as np

from worker.domain import language as language_tools
from worker.domain.audio import decode
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.language import ALIGNER_LANGUAGES, ASR_LANGUAGES, to_wire
from worker.domain.lyrics import (
    anchors,
    draft,
    gapfill,
    interpolate,
    lrc,
    quality,
    rescue,
    text,
    tokens,
)
from worker.domain.lyrics import regions as region_tools
from worker.domain.lyrics.align import RegionAligner
from worker.domain.lyrics.placement import LineTiming, timings_from_words
from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.outcome import (
    LeaseDropped,
    Outcome,
    PermanentFailure,
    Reason,
    TransientFailure,
)
from worker.domain.ports import Engines, EngineUnavailable, Float32Array
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import AudioSection, SyncSection

LANE = "transcribe"
SEPARATION_SAMPLE_RATE = 44_100
SPEECH_SAMPLE_RATE = 16_000
MIN_SPEECH_S = 5.0
STRONG_LANGUAGE_PROB = 0.8
RESCUE_SKIPS = frozenset(
    {Reason.NO_VOCAL_DETECTED, Reason.LYRICS_MISMATCH, Reason.UNSUPPORTED_LANGUAGE}
)
SPOKEN_ALIKE = (frozenset({"zh", "yue"}), frozenset({"ms", "id"}))

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Task:
    track_id: str
    audio_url: str
    reference_text: str
    reference_lines_total: int
    language_hint: str | None


@dataclass(frozen=True)
class Candidate:
    placement: Mapping[int, LineTiming]
    verdict: quality.Verdict


class TranscribeLane:
    def __init__(
        self,
        engines: Engines,
        source: AudioSource,
        workspace: Workspace,
        audio: AudioSection,
        sync: SyncSection,
        sync_version: str,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._source = source
        self._workspace = workspace
        self._audio = audio
        self._sync = sync
        self._sync_version = sync_version
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._sync_lyrics(parse_request(request), deadline)
        except Exception as error:
            return self._failure(error)

    async def _sync_lyrics(self, task: Task, deadline: Deadline) -> Outcome:
        script = text.parse(task.reference_text)
        if not script.sung:
            return self._outcome(Reason.EMPTY_REFERENCE_TEXT, "no sung lines")
        detection = await language_tools.detect(
            task.reference_text, task.language_hint, self._engines, deadline
        )
        track = detection.track
        lines = tokens.tokenize_script(script, detection.lines)
        lines_total = script.lines_total(task.reference_lines_total)
        if unsupported(track, lines, self._sync.unsupported_romanization_share):
            return self._rejected(
                Reason.UNSUPPORTED_LANGUAGE,
                f"language={track}",
                quality.empty_metrics(lines_total, False),
                0.0,
                track,
            )
        vocals, separated = await self._vocals(task, deadline)
        regions = await self._regions(vocals, deadline)
        if region_tools.total_speech_s(regions) < MIN_SPEECH_S:
            return self._rejected(
                Reason.NO_VOCAL_DETECTED,
                f"speech_s={region_tools.total_speech_s(regions):.1f}",
                quality.empty_metrics(lines_total, separated),
                0.0,
                track,
            )
        asr_language = track if self._sync.strategy == "anchored" else None
        if asr_language not in ASR_LANGUAGES:
            asr_language = None
        self._counters.inc(
            "transcribe_strategy_total", strategy="anchored" if asr_language else "global"
        )
        candidate = None
        if asr_language is not None:
            candidate = await self._anchored(
                vocals, regions, lines, asr_language, detection, lines_total, separated, deadline
            )
        if candidate is None:
            candidate = await self._global(vocals, regions, lines, lines_total, separated, deadline)
        elif not candidate.verdict.accepted and candidate.verdict.reason not in RESCUE_SKIPS:
            candidate = await self._rescued(
                candidate, vocals, regions, lines, lines_total, separated, deadline
            )
        return self._publish(script, lines, candidate, track)

    async def _anchored(
        self,
        vocals: Float32Array,
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        track: str,
        detection: language_tools.LanguageDetection,
        lines_total: int,
        separated: bool,
        deadline: Deadline,
    ) -> Candidate | None:
        drafts = await draft.draft_regions(
            self._engines, vocals, regions, track, self._sync.asr, self._counters, deadline
        )
        if all(item.text is None for item in drafts):
            self._counters.inc("draft_fallback_total")
            log.warning("asr drafted no region, aligning globally", extra={"lane": LANE})
            return None
        anchoring = anchors.assign(
            [item.text for item in drafts], lines, regions, self._sync.anchors
        )
        agreement = language_agreement(detection, drafts, regions)
        placement: dict[int, LineTiming] = {}
        if anchoring.placed:
            placement = await self._align_regions(vocals, regions, lines, anchoring, deadline)
            placement.update(await self._fill_gaps(vocals, regions, lines, placement, deadline))
            placement.update(self._interpolate(lines, placement))
        verdict = self._assess(
            placement, regions, lines, lines_total, anchoring.agreement, agreement, separated
        )
        return Candidate(placement, verdict)

    async def _global(
        self,
        vocals: Float32Array,
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        lines_total: int,
        separated: bool,
        deadline: Deadline,
    ) -> Candidate:
        placement, score = await rescue.global_ctc(
            self._engines, vocals, lines, self._counters, deadline
        )
        placement.update(self._interpolate(lines, placement))
        agreement = rescue.ctc_agreement(score, self._sync.quality.min_anchor_agreement)
        verdict = self._assess(placement, regions, lines, lines_total, agreement, None, separated)
        return Candidate(placement, verdict)

    async def _rescued(
        self,
        original: Candidate,
        vocals: Float32Array,
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        lines_total: int,
        separated: bool,
        deadline: Deadline,
    ) -> Candidate:
        if self._sync.rescue_strategy != "global_ctc":
            return original
        placement, score = await rescue.global_ctc(
            self._engines, vocals, lines, self._counters, deadline
        )
        placement.update(self._interpolate(lines, placement))
        agreement = original.verdict.metrics.anchor_agreement or rescue.ctc_agreement(
            score, self._sync.quality.min_anchor_agreement
        )
        verdict = self._assess(
            placement,
            regions,
            lines,
            lines_total,
            agreement,
            original.verdict.metrics.language_agreement,
            separated,
        )
        rescued = Candidate(placement, verdict)
        self._counters.inc("rescue_total", accepted=str(verdict.accepted).lower())
        return better(original, rescued)

    async def _vocals(self, task: Task, deadline: Deadline) -> tuple[Float32Array, bool]:
        with self._workspace.task(f"transcribe-{task.track_id}") as folder:
            audio_path = folder / "audio"
            await self._source.fetch(task.audio_url, audio_path, deadline)
            pcm = await decode.decode(
                audio_path, max_duration_s=self._audio.max_duration_s, deadline=deadline
            )
        if decode.rms_dbfs(decode.to_mono(pcm)) < self._audio.silence_dbfs:
            raise PermanentFailure(Reason.SILENT_AUDIO, "rms below floor")
        stereo = decode.resample(decode.to_stereo(pcm), pcm.sample_rate, SEPARATION_SAMPLE_RATE)
        mix = np.ascontiguousarray(stereo.T, dtype=np.float32)
        deadline.check("separate")
        vocals, separated = await self._separate(mix, deadline)
        mono = vocals.mean(axis=0, dtype=np.float32)
        return decode.resample(mono, SEPARATION_SAMPLE_RATE, SPEECH_SAMPLE_RATE), separated

    async def _separate(self, mix: Float32Array, deadline: Deadline) -> tuple[Float32Array, bool]:
        try:
            vocals = await self._engines.separate(mix, deadline)
        except (EngineUnavailable, PermanentFailure, TransientFailure) as error:
            if isinstance(error, TransientFailure) and error.reason is Reason.DEADLINE_EXCEEDED:
                raise
            self._counters.inc("separation_fallback_total")
            log.warning("separation failed, aligning on the mix", extra={"error": repr(error)})
            return mix, False
        if vocals.shape != mix.shape or not np.all(np.isfinite(vocals)):
            self._counters.inc("separation_fallback_total")
            log.warning(
                "separator output invalid, aligning on the mix", extra={"shape": vocals.shape}
            )
            return mix, False
        return vocals, True

    async def _regions(self, vocals: Float32Array, deadline: Deadline) -> list[Region]:
        deadline.check("vad")
        vad = self._sync.vad
        spans = await self._engines.vad(
            vocals,
            threshold=vad.threshold,
            min_speech_ms=vad.min_speech_ms,
            min_silence_ms=vad.min_silence_ms,
            pad_ms=vad.pad_ms,
            deadline=deadline,
        )
        return region_tools.build(spans, vocals, min_s=vad.region_min_s, max_s=vad.region_max_s)

    async def _align_regions(
        self,
        vocals: Float32Array,
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        anchoring: anchors.Anchoring,
        deadline: Deadline,
    ) -> dict[int, LineTiming]:
        aligner = RegionAligner(self._engines, self._sync, self._counters)
        placement: dict[int, LineTiming] = {}
        for assignment in anchoring.assignments:
            if not assignment.lines:
                continue
            group = [lines[index] for index in assignment.lines]
            span = Region(regions[assignment.region].start_s, regions[assignment.through].end_s)
            result = await aligner.align(vocals, assignment.region, span, group, deadline)
            timings = timings_from_words(
                result.words, result.engine, assignment.region, result.score, 0.0
            )
            placement.update(
                {
                    line: replace(timing, anchor_similarity=anchoring.similarities.get(line, 0.0))
                    for line, timing in timings.items()
                }
            )
        return placement

    async def _fill_gaps(
        self,
        vocals: Float32Array,
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        placement: Mapping[int, LineTiming],
        deadline: Deadline,
    ) -> dict[int, LineTiming]:
        if not self._sync.align.gap_fill:
            return {}
        return await gapfill.fill(
            self._engines,
            vocals,
            regions,
            lines,
            placement,
            max_gap_s=self._sync.align.gap_fill_max_s,
            counters=self._counters,
            deadline=deadline,
        )

    def _interpolate(
        self, lines: Sequence[TokenizedLine], placement: Mapping[int, LineTiming]
    ) -> dict[int, LineTiming]:
        return interpolate.interpolate(
            lines, placement, max_gap_s=self._sync.quality.max_interpolation_gap_s
        )

    def _assess(
        self,
        placement: Mapping[int, LineTiming],
        regions: Sequence[Region],
        lines: Sequence[TokenizedLine],
        lines_total: int,
        anchor_agreement: float,
        language_agreement: float | None,
        separated: bool,
    ) -> quality.Verdict:
        return quality.assess(
            placement,
            regions,
            lines,
            lines_total=lines_total,
            anchor_agreement=anchor_agreement,
            language_agreement=language_agreement,
            separated=separated,
            inside_tolerance_s=self._sync.vad.pad_ms / 1000.0,
            settings=self._sync.quality,
        )

    def _publish(
        self,
        script: text.Script,
        lines: Sequence[TokenizedLine],
        candidate: Candidate,
        track: str | None,
    ) -> Outcome:
        verdict = candidate.verdict
        log.info(
            "lyrics synced",
            extra={
                "lane": LANE,
                "reason": verdict.reason.value if verdict.reason else None,
                "confidence": verdict.confidence,
                **verdict.metrics.to_log(),
            },
        )
        log.debug(
            "line features",
            extra={
                "lane": LANE,
                "line_features": {
                    line: [round(value, 4) for value in row]
                    for line, row in verdict.line_features.items()
                },
            },
        )
        if not verdict.accepted:
            reason = verdict.reason or Reason.LOW_CONFIDENCE
            return self._rejected(
                reason,
                rejection_detail(reason, verdict),
                verdict.metrics,
                verdict.confidence,
                track,
            )
        rendered = lrc.render(script, lines, candidate.placement, pause_s=self._sync.lrc_pause_s)
        fields = self._metric_fields(verdict.metrics, verdict.confidence, track)
        fields["synced_lrc"] = rendered.synced_lrc
        if quality.words_publishable(verdict.metrics, self._sync.quality):
            fields["words"] = rendered.words
        return Outcome.ok(**fields)

    def _rejected(
        self,
        reason: Reason,
        detail: str,
        metrics: quality.Metrics,
        confidence: float,
        track: str | None,
    ) -> Outcome:
        return Outcome.of(reason, detail, **self._metric_fields(metrics, confidence, track))

    def _metric_fields(
        self, metrics: quality.Metrics, confidence: float, track: str | None
    ) -> dict[str, object]:
        return {
            "sync_version": self._sync_version,
            "confidence": round(confidence, 3),
            "placed_share": round(metrics.placed_share, 3),
            "aligned_share": round(metrics.aligned_share, 3),
            "lines_total": metrics.lines_total,
            "lines_unplaced": metrics.lines_unplaced,
            "language": to_wire(track),
        }

    def _outcome(self, reason: Reason, detail: str) -> Outcome:
        return Outcome.of(reason, detail, sync_version=self._sync_version)

    def _failure(self, error: Exception) -> Outcome:
        if isinstance(error, LeaseDropped):
            raise error
        if isinstance(error, PermanentFailure | TransientFailure):
            return error.outcome(sync_version=self._sync_version)
        if isinstance(error, EngineUnavailable):
            self._counters.inc("lane_engine_unavailable_total", lane=LANE, slot=error.slot)
            log.warning("engine unavailable", extra={"slot": error.slot, "state": error.state})
            failure = TransientFailure(
                Reason.ENGINE_CRASHED, f"slot={error.slot} state={error.state}"
            )
            return failure.outcome(sync_version=self._sync_version)
        self._counters.inc("lane_internal_errors_total", lane=LANE, error=type(error).__name__)
        log.error("transcribe failed unexpectedly", extra={"lane": LANE}, exc_info=error)
        failure = TransientFailure(Reason.INTERNAL_ERROR, f"{type(error).__name__}: {error}")
        return failure.outcome(sync_version=self._sync_version)


def parse_request(request: Mapping[str, object]) -> Task:
    track_id = request.get("sc_track_id")
    audio_url = request.get("audio_url")
    reference_text = request.get("reference_text")
    total = request.get("reference_lines_total")
    hint = request.get("language")
    if not isinstance(track_id, str) or not isinstance(audio_url, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, "sc_track_id and audio_url must be strings")
    if not isinstance(reference_text, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, "reference_text must be a string")
    if isinstance(total, bool) or not isinstance(total, int) or total < 1:
        raise PermanentFailure(Reason.INVALID_REQUEST, "reference_lines_total must be >= 1")
    if hint is not None and not isinstance(hint, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, "language must be a string or null")
    return Task(track_id, audio_url, reference_text, total, hint)


def unsupported(track: str | None, lines: Sequence[TokenizedLine], max_missing: float) -> bool:
    if track in ALIGNER_LANGUAGES:
        return False
    return tokens.missing_romanization_share(lines) > max_missing


def language_agreement(
    detection: language_tools.LanguageDetection,
    drafts: Sequence[draft.RegionDraft],
    regions: Sequence[Region],
) -> float | None:
    voted, p_asr = draft.language_vote(drafts, regions)
    track = detection.track
    if voted is None or track is None or track not in ASR_LANGUAGES:
        return None
    if spoken_alike(voted, track):
        return 1.0
    if detection.prob >= STRONG_LANGUAGE_PROB and p_asr >= STRONG_LANGUAGE_PROB:
        return 0.0
    return None


def spoken_alike(first: str, second: str) -> bool:
    return first == second or any({first, second} <= group for group in SPOKEN_ALIKE)


def better(original: Candidate, rescued: Candidate) -> Candidate:
    if rescued.verdict.accepted and not original.verdict.accepted:
        return rescued
    same_status = rescued.verdict.accepted == original.verdict.accepted
    if same_status and rescued.verdict.confidence > original.verdict.confidence:
        return rescued
    return original


def rejection_detail(reason: Reason, verdict: quality.Verdict) -> str:
    metrics = verdict.metrics
    if reason is Reason.LYRICS_MISMATCH:
        return (
            f"anchor_agreement={metrics.anchor_agreement:.2f}"
            f" language_agreement={metrics.language_agreement}"
        )
    if reason is Reason.OUT_OF_ORDER:
        return f"out_of_order_share={metrics.out_of_order_share:.2f}"
    if reason is Reason.TOO_FEW_LINES_PLACED:
        return (
            f"aligned_share={metrics.aligned_share:.2f} placed_share={metrics.placed_share:.2f}"
            f" interpolated_share={metrics.interpolated_share:.2f}"
        )
    if reason is Reason.PLACED_IN_SILENCE:
        return f"inside_share={metrics.inside_share:.2f}"
    return (
        f"confidence={verdict.confidence:.2f} collapsed_share={metrics.collapsed_share:.2f}"
        f" rate_outliers={metrics.rate_outliers:.2f}"
    )
