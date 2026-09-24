from __future__ import annotations

import hashlib
import json
import time
from collections.abc import Mapping, Sequence
from pathlib import Path

import numpy as np
import torch

from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
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
from worker.runtime.protocol import BadInput, ModelSlot, SlotSpec
from worker.settings import Settings

SYNC_SLOTS = ("sep", "asr", "align", "mms")
MEMOIZED_METHODS = frozenset({"separate", "vad", "draft"})
SINGLE_PROCESS_BATCH = {"sep": 1}
LID_PATH = Path.home() / ".cache" / "fasttext" / "lid.176.bin"

Invocation = tuple[Mapping[str, np.ndarray], Mapping[str, object]]


class LocalEngines:
    def __init__(self, settings: Settings, device: str, *, drafts: bool = True) -> None:
        self._settings = settings
        self._device = device
        self._drafts = drafts
        self._slots: dict[str, ModelSlot] = {}
        self._lid = None
        self._memo: dict[str, tuple[str, Invocation]] = {}
        self.gpu_seconds: dict[str, float] = {}
        self.calls: dict[str, int] = {}
        self.memo_hits: dict[str, int] = {}

    def load(self) -> None:
        from worker.models.mms_align import MmsAligner
        from worker.models.qwen_align import QwenAligner
        from worker.models.qwen_asr import QwenAsr
        from worker.models.roformer import RoformerSeparator
        from worker.models.vad import SileroVad

        classes: dict[str, type[ModelSlot]] = {
            "sep": RoformerSeparator,
            "asr": QwenAsr,
            "align": QwenAligner,
            "mms": MmsAligner,
        }
        for name in SYNC_SLOTS:
            if name == "asr" and not self._drafts:
                continue
            slot = classes[name]()
            slot.load(self._spec(name, self._device))
            slot.warmup()
            self._slots[name] = slot
        vad = SileroVad()
        vad.load(self._spec("cpu-tools", "cpu"))
        vad.warmup()
        self._slots["cpu-tools"] = vad
        if LID_PATH.is_file():
            import fasttext

            self._lid = fasttext.load_model(str(LID_PATH))

    def unload(self) -> None:
        for slot in self._slots.values():
            slot.unload()
        self._slots.clear()

    async def separate(self, mix_stereo_44k: Float32Array, deadline: Deadline) -> Float32Array:
        arrays, _ = self._invoke("sep", "separate", {"mix": mix_stereo_44k}, {})
        return np.asarray(arrays["vocals"], dtype=np.float32)

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
        _, result = self._invoke(
            "cpu-tools",
            "vad",
            {"vocals": vocals_mono_16k},
            {
                "threshold": threshold,
                "min_speech_ms": min_speech_ms,
                "min_silence_ms": min_silence_ms,
                "pad_ms": pad_ms,
            },
        )
        return [Span(float(start), float(end)) for start, end in result["spans"]]

    async def draft(
        self,
        clips_mono_16k: Sequence[Float32Array],
        language: str,
        max_new_tokens: Sequence[int],
        repetition_penalty: float,
        deadline: Deadline,
    ) -> Sequence[Draft]:
        if not self._drafts:
            return [Draft("", None, 0.0) for _ in clips_mono_16k]
        arrays = {f"clip_{index}": clip for index, clip in enumerate(clips_mono_16k)}
        _, result = self._invoke(
            "asr",
            "draft",
            arrays,
            {
                "language": language,
                "max_new_tokens": list(max_new_tokens),
                "repetition_penalty": repetition_penalty,
            },
        )
        return [
            Draft(str(item["text"]), item["language"], float(item["language_prob"]))
            for item in result["drafts"]
        ]

    async def align(
        self,
        clip_mono_16k: Float32Array,
        tokens: Sequence[str],
        language: str,
        deadline: Deadline,
    ) -> Alignment:
        _, result = self._invoke(
            "align",
            "align",
            {"clip": clip_mono_16k},
            {"tokens": list(tokens), "language": language},
        )
        return alignment_of(result)

    async def ctc_align(
        self,
        clip_mono_16k: Float32Array,
        romanized_tokens: Sequence[str],
        deadline: Deadline,
    ) -> Alignment:
        _, result = self._invoke(
            "mms", "ctc_align", {"clip": clip_mono_16k}, {"tokens": list(romanized_tokens)}
        )
        return alignment_of(result)

    async def detect_language(
        self, lines: Sequence[str], deadline: Deadline
    ) -> Sequence[Sequence[LanguageGuess]]:
        if self._lid is None:
            return [[] for _ in lines]
        guesses: list[list[LanguageGuess]] = []
        for line in lines:
            labels, scores = self._lid.predict(line.replace("\n", " "), k=3)
            guesses.append(
                [
                    LanguageGuess(str(label).replace("__label__", ""), float(score))
                    for label, score in zip(labels, scores, strict=True)
                ]
            )
        return guesses

    async def embed_audio(self, windows: Float32Array, deadline: Deadline) -> AudioEmbeddings:
        raise NotImplementedError("eval engines serve the sync lane only")

    async def embed_text(
        self, texts: Sequence[str], kind: TextKind, deadline: Deadline
    ) -> Float32Array:
        raise NotImplementedError("eval engines serve the sync lane only")

    async def embed_text_mulan(self, texts: Sequence[str], deadline: Deadline) -> Float32Array:
        raise NotImplementedError("eval engines serve the sync lane only")

    async def fingerprint(
        self,
        pcm_interleaved: Int16Array,
        sample_rate: int,
        channels: int,
        deadline: Deadline,
    ) -> str | None:
        raise NotImplementedError("eval engines serve the sync lane only")

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
        raise NotImplementedError("eval engines serve the sync lane only")

    async def generate(
        self,
        prompt: str,
        schema: Mapping[str, object],
        max_new_tokens: int,
        deadline: Deadline,
    ) -> str:
        raise NotImplementedError("eval engines serve the sync lane only")

    def _invoke(
        self,
        slot: str,
        method: str,
        arrays: Mapping[str, np.ndarray],
        args: Mapping[str, object],
    ) -> Invocation:
        key = invocation_key(arrays, args) if method in MEMOIZED_METHODS else None
        remembered = self._memo.get(method)
        if key is not None and remembered is not None and remembered[0] == key:
            self.memo_hits[method] = self.memo_hits.get(method, 0) + 1
            return remembered[1]
        started = time.perf_counter()
        try:
            result = self._slots[slot].invoke(method, arrays, args)
        except BadInput as error:
            raise PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"{slot}: {error}") from error
        except torch.OutOfMemoryError as error:
            torch.cuda.empty_cache()
            raise TransientFailure(Reason.OUT_OF_MEMORY, f"{slot}: {error}") from error
        except Exception as error:
            raise TransientFailure(Reason.INTERNAL_ERROR, f"{slot}: {error!r}") from error
        finally:
            elapsed = time.perf_counter() - started
            self.gpu_seconds[slot] = self.gpu_seconds.get(slot, 0.0) + elapsed
            self.calls[slot] = self.calls.get(slot, 0) + 1
        if key is not None:
            self._memo[method] = (key, result)
        return result

    def _spec(self, name: str, device: str) -> SlotSpec:
        slot = self._settings.slots[name]
        return SlotSpec(
            name=name,
            loader="",
            model=slot.model,
            revision=slot.revision,
            device=device,
            max_batch=SINGLE_PROCESS_BATCH.get(name, slot.max_batch),
            max_wait_ms=slot.max_wait_ms,
        )


def invocation_key(arrays: Mapping[str, np.ndarray], args: Mapping[str, object]) -> str:
    digest = hashlib.sha256(json.dumps(args, sort_keys=True, default=str).encode())
    for name in sorted(arrays):
        array = np.ascontiguousarray(arrays[name])
        digest.update(name.encode())
        digest.update(str(array.shape).encode())
        digest.update(array.tobytes())
    return digest.hexdigest()


def alignment_of(result: Mapping[str, object]) -> Alignment:
    spans = tuple(
        TokenSpan(float(start), float(end), float(score))
        for start, end, score in list(result["spans"])
    )
    return Alignment(spans, float(result["score"]))
