from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, Protocol

import numpy as np
from numpy.typing import NDArray

from worker.domain.deadline import Deadline

Float32Array = NDArray[np.float32]
Int16Array = NDArray[np.int16]

TextKind = Literal["document", "query"]
Grounding = Callable[[Mapping[str, object]], bool]


class EngineUnavailable(Exception):
    def __init__(self, slot: str, state: str) -> None:
        super().__init__(f"slot {slot} is {state}")
        self.slot = slot
        self.state = state


@dataclass(frozen=True)
class AudioEmbeddings:
    mert: Float32Array
    clap: Float32Array


@dataclass(frozen=True)
class Span:
    start_s: float
    end_s: float


@dataclass(frozen=True)
class Draft:
    text: str
    language: str | None
    language_prob: float


@dataclass(frozen=True)
class TokenSpan:
    start_s: float
    end_s: float
    score: float


@dataclass(frozen=True)
class Alignment:
    spans: Sequence[TokenSpan]
    score: float


@dataclass(frozen=True)
class LanguageGuess:
    code: str
    prob: float


@dataclass(frozen=True)
class CollabTraining:
    sessions: int
    vocab: int
    points_count: int
    hr_at_20: float
    popularity_hr_at_20: float


class Engines(Protocol):
    async def embed_audio(self, windows: Float32Array, deadline: Deadline) -> AudioEmbeddings: ...

    async def embed_text(
        self, texts: Sequence[str], kind: TextKind, deadline: Deadline
    ) -> Float32Array: ...

    async def embed_text_mulan(self, texts: Sequence[str], deadline: Deadline) -> Float32Array: ...

    async def separate(self, mix_stereo_44k: Float32Array, deadline: Deadline) -> Float32Array: ...

    async def vad(
        self,
        vocals_mono_16k: Float32Array,
        *,
        threshold: float,
        min_speech_ms: int,
        min_silence_ms: int,
        pad_ms: int,
        deadline: Deadline,
    ) -> Sequence[Span]: ...

    async def draft(
        self,
        clips_mono_16k: Sequence[Float32Array],
        language: str,
        max_new_tokens: Sequence[int],
        repetition_penalty: float,
        deadline: Deadline,
    ) -> Sequence[Draft]: ...

    async def align(
        self,
        clip_mono_16k: Float32Array,
        tokens: Sequence[str],
        language: str,
        deadline: Deadline,
    ) -> Alignment: ...

    async def ctc_align(
        self,
        clip_mono_16k: Float32Array,
        romanized_tokens: Sequence[str],
        deadline: Deadline,
    ) -> Alignment: ...

    async def detect_language(
        self, lines: Sequence[str], deadline: Deadline
    ) -> Sequence[Sequence[LanguageGuess]]: ...

    async def fingerprint(
        self,
        pcm_interleaved: Int16Array,
        sample_rate: int,
        channels: int,
        deadline: Deadline,
    ) -> str | None: ...

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
    ) -> CollabTraining: ...

    async def generate(
        self,
        prompt: str,
        schema: Mapping[str, object],
        max_new_tokens: int,
        deadline: Deadline,
    ) -> str: ...


class Refiner(Protocol):
    async def refine(
        self,
        prompt: str,
        schema: Mapping[str, object],
        grounded: Grounding,
        deadline: Deadline,
    ) -> Mapping[str, object] | None: ...


class BlobStore(Protocol):
    async def get(self, bucket: str, name: str, into: Path, deadline: Deadline) -> int: ...

    async def put(self, bucket: str, name: str, source: Path, deadline: Deadline) -> None: ...

    async def delete(self, bucket: str, name: str, deadline: Deadline) -> None: ...
