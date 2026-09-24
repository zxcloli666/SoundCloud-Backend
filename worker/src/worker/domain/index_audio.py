from __future__ import annotations

import logging
from collections.abc import Mapping

from worker.domain import embedding
from worker.domain.audio import decode, windows
from worker.domain.audio.source import AudioSource
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason, TransientFailure
from worker.domain.ports import Engines, EngineUnavailable, Float32Array
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import AudioSection

EMBED_SAMPLE_RATE = 24_000
MERT_DIM = 1024
CLAP_DIM = 512
FINGERPRINT_MIN_S = 10.0
FINGERPRINT_CHARS = 64
LANE = "audio"

log = logging.getLogger(__name__)


class IndexAudioLane:
    def __init__(
        self,
        engines: Engines,
        source: AudioSource,
        workspace: Workspace,
        settings: AudioSection,
        counters: Counters,
    ) -> None:
        self._engines = engines
        self._source = source
        self._workspace = workspace
        self._settings = settings
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._index(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _index(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        track_id = embedding.text_field(request, "sc_track_id")
        url = embedding.text_field(request, "s3_url")
        with self._workspace.task(f"index_audio-{track_id}") as folder:
            audio_path = folder / "audio"
            await self._source.fetch(url, audio_path, deadline)
            pcm = await decode.decode(
                audio_path, max_duration_s=self._settings.max_duration_s, deadline=deadline
            )
        if pcm.duration_s < self._settings.min_duration_s:
            return Outcome.of(Reason.AUDIO_TOO_SHORT, f"duration_s={pcm.duration_s:.2f}")
        mono = decode.to_mono(pcm)
        loudness = decode.rms_dbfs(mono)
        if loudness < self._settings.silence_dbfs:
            return Outcome.of(Reason.SILENT_AUDIO, f"rms_dbfs={loudness:.1f}")
        clips = embedding_clips(mono, pcm.sample_rate)
        deadline.check("embed_audio")
        vectors = await self._engines.embed_audio(clips, deadline)
        mert = embedding.pooled(vectors.mert, MERT_DIM, "muq", self._counters)
        clap = embedding.pooled(vectors.clap, CLAP_DIM, "mulan", self._counters)
        fingerprint = await self._fingerprint(pcm, deadline)
        return Outcome.ok(mert=mert, clap=clap, fingerprint=fingerprint)

    async def _fingerprint(self, pcm: decode.Pcm, deadline: Deadline) -> str | None:
        if pcm.duration_s < FINGERPRINT_MIN_S:
            return None
        samples = decode.pcm16_interleaved(pcm, self._settings.fingerprint_s)
        try:
            value = await self._engines.fingerprint(
                samples, pcm.sample_rate, pcm.channels, deadline
            )
        except (TransientFailure, PermanentFailure, EngineUnavailable) as error:
            self._counters.inc("fingerprint_failures_total", error=type(error).__name__)
            log.warning("fingerprint failed, publishing null", extra={"error": repr(error)})
            return None
        if not value:
            self._counters.inc("fingerprint_unavailable_total")
            log.warning("fingerprint engine returned nothing, publishing null")
            return None
        return value[:FINGERPRINT_CHARS]


def embedding_clips(mono: Float32Array, sample_rate: int) -> Float32Array:
    signal = decode.resample(mono, sample_rate, EMBED_SAMPLE_RATE)
    chosen = windows.select_windows(signal, EMBED_SAMPLE_RATE)
    return windows.cut(signal, EMBED_SAMPLE_RATE, chosen)
