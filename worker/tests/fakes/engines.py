from __future__ import annotations

import hashlib
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path

import numpy as np

from worker.domain.deadline import Deadline
from worker.domain.ports import (
    Alignment,
    AudioEmbeddings,
    CollabTraining,
    Draft,
    Float32Array,
    Int16Array,
    LanguageGuess,
    Span,
    TextKind,
    TokenSpan,
)

MERT_DIM = 1024
CLAP_DIM = 512
TEXT_DIM = 1024
MULAN_TEXT_DIM = 512


def unit_vector(seed: bytes, dim: int) -> Float32Array:
    digest = hashlib.sha256(seed).digest()
    generator = np.random.default_rng(int.from_bytes(digest[:8], "little"))
    vector = generator.standard_normal(dim).astype(np.float32)
    return vector / np.linalg.norm(vector)


class FakeEngines:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, object]]] = []
        self.overrides: dict[str, Callable[..., object]] = {}
        self.regions: list[Span] = [Span(0.5, 8.0), Span(9.0, 20.0)]
        self.drafts: list[Draft] = []
        self.languages: dict[str, list[LanguageGuess]] = {}
        self.default_language = [LanguageGuess("en", 0.95)]
        self.fingerprint_value: str | None = "AQADtEmUfake"
        self.collab_result = CollabTraining(120, 300, 300, 0.30, 0.20)
        self.generated = "{}"

    def fail(self, method: str, error: Exception) -> None:
        def raiser(**kwargs: object) -> object:
            raise error

        self.overrides[method] = raiser

    def _record(self, method: str, **kwargs: object) -> object | None:
        self.calls.append((method, kwargs))
        override = self.overrides.get(method)
        return override(**kwargs) if override is not None else None

    async def embed_audio(self, windows: Float32Array, deadline: Deadline) -> AudioEmbeddings:
        deadline.check("embed_audio")
        forced = self._record("embed_audio", windows=windows)
        if forced is not None:
            return forced
        rows = windows.shape[0]
        mert = np.stack([unit_vector(windows[i].tobytes(), MERT_DIM) for i in range(rows)])
        clap = np.stack([unit_vector(windows[i].tobytes() + b"c", CLAP_DIM) for i in range(rows)])
        return AudioEmbeddings(mert=mert, clap=clap)

    async def embed_text(
        self, texts: Sequence[str], kind: TextKind, deadline: Deadline
    ) -> Float32Array:
        deadline.check("embed_text")
        forced = self._record("embed_text", texts=texts, kind=kind)
        if forced is not None:
            return forced
        return np.stack([unit_vector(f"{kind}:{text}".encode(), TEXT_DIM) for text in texts])

    async def embed_text_mulan(self, texts: Sequence[str], deadline: Deadline) -> Float32Array:
        deadline.check("embed_text_mulan")
        forced = self._record("embed_text_mulan", texts=texts)
        if forced is not None:
            return forced
        return np.stack([unit_vector(f"mulan:{text}".encode(), MULAN_TEXT_DIM) for text in texts])

    async def separate(self, mix_stereo_44k: Float32Array, deadline: Deadline) -> Float32Array:
        deadline.check("separate")
        forced = self._record("separate", mix=mix_stereo_44k)
        if forced is not None:
            return forced
        return mix_stereo_44k

    async def vad(
        self,
        vocals_mono_16k: Float32Array,
        *,
        threshold: float,
        min_speech_ms: int,
        min_silence_ms: int,
        pad_ms: int,
        deadline: Deadline,
    ) -> Sequence[Span]:
        deadline.check("vad")
        forced = self._record(
            "vad",
            samples=int(vocals_mono_16k.shape[0]),
            threshold=threshold,
            min_speech_ms=min_speech_ms,
            min_silence_ms=min_silence_ms,
            pad_ms=pad_ms,
        )
        if forced is not None:
            return forced
        duration = vocals_mono_16k.shape[0] / 16_000
        return [span for span in self.regions if span.end_s <= duration]

    async def draft(
        self,
        clips_mono_16k: Sequence[Float32Array],
        language: str,
        max_new_tokens: Sequence[int],
        repetition_penalty: float,
        deadline: Deadline,
    ) -> Sequence[Draft]:
        deadline.check("draft")
        forced = self._record(
            "draft",
            clips=len(clips_mono_16k),
            language=language,
            max_new_tokens=list(max_new_tokens),
            repetition_penalty=repetition_penalty,
        )
        if forced is not None:
            return forced
        if self.drafts:
            return list(self.drafts[: len(clips_mono_16k)])
        return [Draft("", language, 0.0) for _ in clips_mono_16k]

    async def align(
        self,
        clip_mono_16k: Float32Array,
        tokens: Sequence[str],
        language: str,
        deadline: Deadline,
    ) -> Alignment:
        deadline.check("align")
        forced = self._record("align", tokens=list(tokens), language=language)
        if forced is not None:
            return forced
        return even_alignment(clip_mono_16k.shape[0] / 16_000, len(tokens), 0.9)

    async def ctc_align(
        self,
        clip_mono_16k: Float32Array,
        romanized_tokens: Sequence[str],
        deadline: Deadline,
    ) -> Alignment:
        deadline.check("ctc_align")
        forced = self._record("ctc_align", tokens=list(romanized_tokens))
        if forced is not None:
            return forced
        return even_alignment(clip_mono_16k.shape[0] / 16_000, len(romanized_tokens), 0.8)

    async def detect_language(
        self, lines: Sequence[str], deadline: Deadline
    ) -> Sequence[Sequence[LanguageGuess]]:
        deadline.check("detect_language")
        forced = self._record("detect_language", lines=list(lines))
        if forced is not None:
            return forced
        return [self.languages.get(line, self.default_language) for line in lines]

    async def fingerprint(
        self,
        pcm_interleaved: Int16Array,
        sample_rate: int,
        channels: int,
        deadline: Deadline,
    ) -> str | None:
        deadline.check("fingerprint")
        forced = self._record(
            "fingerprint",
            samples=int(pcm_interleaved.shape[0]),
            sample_rate=sample_rate,
            channels=channels,
        )
        if forced is not None:
            return forced
        return self.fingerprint_value

    async def train_collab(
        self,
        sessions_path: Path,
        vectors_path: Path,
        *,
        min_count: int,
        window: int,
        epochs: int,
        negative: int,
        deadline: Deadline,
    ) -> CollabTraining:
        deadline.check("train_collab")
        forced = self._record(
            "train_collab",
            sessions_path=sessions_path,
            vectors_path=vectors_path,
            min_count=min_count,
            window=window,
            epochs=epochs,
            negative=negative,
        )
        if forced is not None:
            return forced
        vectors_path.write_bytes(b'{"dim":128,"points":[],"metrics":{}}')
        return self.collab_result

    async def generate(
        self,
        prompt: str,
        schema: Mapping[str, object],
        max_new_tokens: int,
        deadline: Deadline,
    ) -> str:
        deadline.check("generate")
        forced = self._record(
            "generate", prompt=prompt, schema=dict(schema), max_new_tokens=max_new_tokens
        )
        if forced is not None:
            return forced
        return self.generated


def even_alignment(duration_s: float, tokens: int, score: float) -> Alignment:
    if tokens == 0:
        return Alignment((), score)
    step = duration_s / tokens
    spans = tuple(
        TokenSpan(round(i * step, 3), round((i + 1) * step - 0.01, 3), score) for i in range(tokens)
    )
    return Alignment(spans, score)
