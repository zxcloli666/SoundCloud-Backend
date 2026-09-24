from __future__ import annotations

import asyncio
import logging
import os
from collections.abc import Callable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import astuple, replace
from pathlib import Path

import numpy as np

from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.domain.ports import (
    Alignment,
    AudioEmbeddings,
    CollabTraining,
    Draft,
    EngineUnavailable,
    Float32Array,
    Int16Array,
    LanguageGuess,
    Span,
    TextKind,
    TokenSpan,
)
from worker.domain.taste import BASELINES as TASTE_BASELINES
from worker.domain.taste import TasteComparison, TasteScores, TasteTraining
from worker.runtime import shm
from worker.runtime.batcher import Batcher
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    DeadlineExceeded,
    EngineClient,
    EngineCrashed,
    EngineError,
    EngineKilled,
    SlotUnavailable,
    next_message_id,
)
from worker.runtime.protocol import Arrays, Call, ErrorKind
from worker.runtime.supervisor import Supervisor

BATCHED_SLOTS = ("muq", "mulan", "text")
TASTE_SLOT = "train-taste"
TEXT_BYTES_PER_TOKEN = 4

log = logging.getLogger(__name__)

Result = tuple[dict[str, np.ndarray], dict[str, object]]


class EngineSlots:
    def __init__(
        self,
        supervisor: Supervisor,
        batchers: Mapping[str, Batcher],
        max_batch: Mapping[str, int],
        before_queue: Callable[[], None],
    ) -> None:
        self._supervisor = supervisor
        self._batchers = batchers
        self._max_batch = max_batch
        self._before_queue = before_queue
        self._orphans: set[asyncio.Task[Result]] = set()

    def max_batch(self, slot: str) -> int:
        return max(1, self._max_batch.get(slot, 1))

    async def batched(
        self,
        slot: str,
        method: str,
        arrays: Arrays,
        args: Mapping[str, object],
        deadline: Deadline,
        *,
        bad_input: Reason = Reason.MODEL_OUTPUT_INVALID,
        costs: Sequence[int] | None = None,
        priority: bool = False,
    ) -> Result:
        batcher = self._batchers.get(slot)
        if batcher is None:
            raise EngineUnavailable(slot, "unknown")
        self._before_queue()
        with translated(slot, bad_input):
            return await batcher.submit(
                method, arrays, args, deadline.at, costs=costs, priority=priority
            )

    async def direct(
        self,
        slot: str,
        method: str,
        arrays: Arrays,
        args: Mapping[str, object],
        deadline: Deadline,
        *,
        bad_input: Reason = Reason.MODEL_OUTPUT_INVALID,
    ) -> Result:
        self._before_queue()
        with translated(slot, bad_input):
            client = await self._supervisor.acquire(slot, deadline.at)
            call = asyncio.ensure_future(self._call(client, slot, method, arrays, args, deadline))
            try:
                return await asyncio.shield(call)
            except asyncio.CancelledError:
                self._orphans.add(call)
                call.add_done_callback(self._orphan_finished)
                raise

    async def _call(
        self,
        client: EngineClient,
        slot: str,
        method: str,
        arrays: Arrays,
        args: Mapping[str, object],
        deadline: Deadline,
    ) -> Result:
        call_id = next_message_id()
        blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
        try:
            refs = blocks.share(arrays)
            reply = await client.call(Call(call_id, slot, method, deadline.at, refs, dict(args)))
        finally:
            blocks.release()
            self._supervisor.release(client)
        if reply.error_kind is not None:
            if reply.error_kind is ErrorKind.OOM:
                self._supervisor.report_oom(slot, client)
            raise EngineError(reply.error_kind, reply.error or "")
        try:
            outputs = shm.take_all(reply.arrays)
        except OSError as error:
            raise EngineCrashed(client.name, f"reply arrays unreadable: {error}") from error
        return outputs, dict(reply.result)

    def _orphan_finished(self, call: asyncio.Task[Result]) -> None:
        self._orphans.discard(call)
        if call.cancelled():
            return
        error = call.exception()
        if error is not None:
            log.warning(
                "abandoned engine call failed", extra={"error": f"{type(error).__name__}: {error}"}
            )


class RuntimeEngines:
    def __init__(self, slots: EngineSlots, *, priority: bool) -> None:
        self._slots = slots
        self._priority = priority

    async def embed_audio(self, windows: Float32Array, deadline: Deadline) -> AudioEmbeddings:
        arrays = {"windows": windows}
        (mert, _), (clap, _) = await asyncio.gather(
            self._slots.batched("muq", "embed", arrays, {}, deadline),
            self._slots.batched("mulan", "embed_audio", arrays, {}, deadline),
        )
        return AudioEmbeddings(
            mert=float32(mert, "vectors", "muq"), clap=float32(clap, "vectors", "mulan")
        )

    async def embed_text(
        self, texts: Sequence[str], kind: TextKind, deadline: Deadline
    ) -> Float32Array:
        arrays, _ = await self._slots.batched(
            "text",
            "embed",
            {},
            {"texts": list(texts), "kind": kind},
            deadline,
            bad_input=Reason.TEXT_TOO_LONG_FOR_MODEL,
            costs=[token_estimate(text) for text in texts],
            priority=self._priority,
        )
        return float32(arrays, "vectors", "text")

    async def embed_text_mulan(self, texts: Sequence[str], deadline: Deadline) -> Float32Array:
        arrays, _ = await self._slots.batched(
            "mulan",
            "embed_text",
            {},
            {"texts": list(texts)},
            deadline,
            bad_input=Reason.TEXT_TOO_LONG_FOR_MODEL,
            priority=self._priority,
        )
        return float32(arrays, "vectors", "mulan")

    async def separate(self, mix_stereo_44k: Float32Array, deadline: Deadline) -> Float32Array:
        arrays, _ = await self._slots.direct(
            "sep", "separate", {"mix": mix_stereo_44k}, {}, deadline
        )
        return float32(arrays, "vocals", "sep")

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
        _, result = await self._slots.direct(
            "vad",
            "vad",
            {"vocals": vocals_mono_16k},
            {
                "threshold": threshold,
                "min_speech_ms": min_speech_ms,
                "min_silence_ms": min_silence_ms,
                "pad_ms": pad_ms,
            },
            deadline,
        )
        return [Span(start, end) for start, end in number_rows(result, "spans", "vad", 2)]

    async def draft(
        self,
        clips_mono_16k: Sequence[Float32Array],
        language: str,
        max_new_tokens: Sequence[int],
        repetition_penalty: float,
        deadline: Deadline,
    ) -> Sequence[Draft]:
        size = self._slots.max_batch("asr")
        chunks = [
            range(start, min(start + size, len(clips_mono_16k)))
            for start in range(0, len(clips_mono_16k), size)
        ]
        drafted = await asyncio.gather(
            *(
                self._draft_chunk(
                    [clips_mono_16k[index] for index in chunk],
                    language,
                    [max_new_tokens[index] for index in chunk],
                    repetition_penalty,
                    deadline,
                )
                for chunk in chunks
            )
        )
        return [draft for part in drafted for draft in part]

    async def align(
        self,
        clip_mono_16k: Float32Array,
        tokens: Sequence[str],
        language: str,
        deadline: Deadline,
    ) -> Alignment:
        _, result = await self._slots.direct(
            "align",
            "align",
            {"clip": clip_mono_16k},
            {"tokens": list(tokens), "language": language},
            deadline,
        )
        return alignment(result, "align")

    async def ctc_align(
        self,
        clip_mono_16k: Float32Array,
        romanized_tokens: Sequence[str],
        deadline: Deadline,
    ) -> Alignment:
        _, result = await self._slots.direct(
            "mms",
            "ctc_align",
            {"clip": clip_mono_16k},
            {"tokens": list(romanized_tokens)},
            deadline,
        )
        return alignment(result, "mms")

    async def detect_language(
        self, lines: Sequence[str], deadline: Deadline
    ) -> Sequence[Sequence[LanguageGuess]]:
        if not lines:
            return []
        _, result = await self._slots.direct(
            "lid", "detect_language", {}, {"lines": list(lines)}, deadline
        )
        rows = listed(result, "guesses", "lid")
        if len(rows) != len(lines):
            raise invalid("lid", f"{len(rows)} guess rows for {len(lines)} lines")
        return [[guess(item) for item in listed_value(row, "lid")] for row in rows]

    async def fingerprint(
        self,
        pcm_interleaved: Int16Array,
        sample_rate: int,
        channels: int,
        deadline: Deadline,
    ) -> str | None:
        _, result = await self._slots.direct(
            "fingerprint",
            "fingerprint",
            {"pcm": pcm_interleaved},
            {"sample_rate": sample_rate, "channels": channels},
            deadline,
        )
        value = result.get("fingerprint")
        if value is not None and not isinstance(value, str):
            raise invalid("fingerprint", f"fingerprint is {type(value).__name__}")
        return value

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
        _, result = await self._slots.direct(
            "train-collab",
            "train",
            {},
            {
                "sessions_path": str(sessions_path),
                "vectors_path": str(vectors_path),
                "min_count": min_count,
                "window": window,
                "epochs": epochs,
                "negative": negative,
            },
            deadline,
            bad_input=Reason.INVALID_REQUEST,
        )
        return CollabTraining(
            sessions=int(number(result, "sessions", "train-collab")),
            vocab=int(number(result, "vocab", "train-collab")),
            points_count=int(number(result, "points_count", "train-collab")),
            hr_at_20=number(result, "hr_at_20", "train-collab"),
            popularity_hr_at_20=number(result, "popularity_hr_at_20", "train-collab"),
        )

    async def train_taste(
        self,
        input_path: Path,
        artifact_path: Path,
        tower_path: Path,
        *,
        previous_path: Path | None,
        epochs: int,
        batch_size: int,
        negatives: int,
        seed: int,
        min_users: int,
        budget_s: int,
        trained_at: int,
        deadline: Deadline,
    ) -> TasteTraining:
        args: dict[str, object] = {
            "input_path": str(input_path),
            "artifact_path": str(artifact_path),
            "tower_path": str(tower_path),
            "epochs": epochs,
            "batch_size": batch_size,
            "negatives": negatives,
            "seed": seed,
            "min_users": min_users,
            "budget_s": budget_s,
            "trained_at": trained_at,
        }
        if previous_path is not None:
            args["previous_path"] = str(previous_path)
        _, result = await self._slots.direct(
            TASTE_SLOT, "train", {}, args, deadline, bad_input=Reason.INVALID_REQUEST
        )
        return taste_training(result)

    async def generate(
        self,
        prompt: str,
        schema: Mapping[str, object],
        max_new_tokens: int,
        deadline: Deadline,
    ) -> str:
        _, result = await self._slots.direct(
            "llm-local",
            "generate",
            {},
            {"prompt": prompt, "schema": dict(schema), "max_new_tokens": max_new_tokens},
            deadline,
        )
        text = result.get("text")
        if not isinstance(text, str):
            raise invalid("llm-local", "text is not a string")
        return text

    async def _draft_chunk(
        self,
        clips: Sequence[Float32Array],
        language: str,
        max_new_tokens: Sequence[int],
        repetition_penalty: float,
        deadline: Deadline,
    ) -> list[Draft]:
        _, result = await self._slots.direct(
            "asr",
            "draft",
            {f"clip_{index}": clip for index, clip in enumerate(clips)},
            {
                "language": language,
                "max_new_tokens": list(max_new_tokens),
                "repetition_penalty": repetition_penalty,
            },
            deadline,
        )
        rows = listed(result, "drafts", "asr")
        if len(rows) != len(clips):
            raise invalid("asr", f"{len(rows)} drafts for {len(clips)} clips")
        return [draft(row) for row in rows]


@contextmanager
def translated(slot: str, bad_input: Reason) -> Iterator[None]:
    try:
        yield
    except EngineError as error:
        detail = f"slot={slot} {error.message}"
        if error.kind is ErrorKind.BAD_INPUT:
            raise PermanentFailure(bad_input, detail) from error
        if error.kind is ErrorKind.OOM:
            raise TransientFailure(Reason.OUT_OF_MEMORY, detail) from error
        raise TransientFailure(Reason.INTERNAL_ERROR, detail) from error
    except DeadlineExceeded as error:
        raise TransientFailure(
            Reason.DEADLINE_EXCEEDED, f"slot={slot} stage={error.stage}"
        ) from error
    except EngineKilled as error:
        reason = (
            Reason.DEADLINE_EXCEEDED if error.cause == CAUSE_DEADLINE else Reason.ENGINE_CRASHED
        )
        raise TransientFailure(reason, f"slot={slot} killed={error.cause}") from error
    except EngineCrashed as error:
        raise TransientFailure(Reason.ENGINE_CRASHED, f"slot={slot} {error.detail}") from error
    except SlotUnavailable as error:
        raise EngineUnavailable(error.slot, error.state) from error


def token_estimate(text: str) -> int:
    return max(1, len(text.encode("utf-8")) // TEXT_BYTES_PER_TOKEN)


def float32(arrays: Mapping[str, np.ndarray], key: str, slot: str) -> Float32Array:
    array = arrays.get(key)
    if array is None:
        raise invalid(slot, f"reply has no array {key!r}")
    return np.ascontiguousarray(array, dtype=np.float32)


def alignment(result: Mapping[str, object], slot: str) -> Alignment:
    spans = [
        TokenSpan(start, end, score) for start, end, score in number_rows(result, "spans", slot, 3)
    ]
    return Alignment(spans=spans, score=number(result, "score", slot))


def taste_training(result: Mapping[str, object]) -> TasteTraining:
    budget_spent = result.get("budget_spent")
    previous_state = result.get("previous_state")
    if not isinstance(budget_spent, bool) or not isinstance(previous_state, str):
        raise invalid(TASTE_SLOT, "result lacks budget_spent or previous_state")
    untrained = TasteTraining(
        users_count=int(number(result, "users_count", TASTE_SLOT)),
        items_count=int(number(result, "items_count", TASTE_SLOT)),
        test_users=int(number(result, "test_users", TASTE_SLOT)),
        epochs_done=int(number(result, "epochs_done", TASTE_SLOT)),
        evaluated_users=int(number(result, "evaluated_users", TASTE_SLOT)),
        steps=int(number(result, "steps", TASTE_SLOT)),
        budget_spent=budget_spent,
        previous_state=previous_state,
    )
    if result.get("trained") is not True:
        return untrained
    version = result.get("version")
    tower_object = result.get("tower_object")
    if not isinstance(version, str) or not isinstance(tower_object, str):
        raise invalid(TASTE_SLOT, "trained result lacks version or tower_object")
    metrics = result.get("metrics")
    if not isinstance(metrics, Mapping):
        raise invalid(TASTE_SLOT, "trained result lacks metrics")
    baselines = metrics.get("baselines")
    if not isinstance(baselines, Mapping):
        raise invalid(TASTE_SLOT, "metrics lack baselines")
    return replace(
        untrained,
        version=version,
        tower_object=tower_object,
        model=taste_scores(metrics.get("model")),
        baselines={name: taste_scores(baselines.get(name)) for name in TASTE_BASELINES},
        previous=taste_comparison(metrics.get("previous")),
    )


def taste_comparison(value: object) -> TasteComparison | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise invalid(TASTE_SLOT, "previous comparison is not an object")
    return TasteComparison(
        users=int(number(value, "users", TASTE_SLOT)),
        model=taste_scores(value.get("model")),
        previous=taste_scores(value.get("previous")),
    )


def taste_scores(value: object) -> TasteScores:
    if not isinstance(value, Mapping):
        raise invalid(TASTE_SLOT, "scores are not an object")
    scores = TasteScores(
        recall_at_50=number(value, "recall_at_50", TASTE_SLOT),
        ndcg_at_20=number(value, "ndcg_at_20", TASTE_SLOT),
        cold_recall_at_50=number(value, "cold_recall_at_50", TASTE_SLOT),
        coverage_at_50=number(value, "coverage_at_50", TASTE_SLOT),
    )
    if any(not 0.0 <= share <= 1.0 for share in astuple(scores)):
        raise invalid(TASTE_SLOT, f"scores out of [0, 1]: {scores}")
    return scores


def draft(row: object) -> Draft:
    if not isinstance(row, Mapping):
        raise invalid("asr", "draft is not an object")
    text = row.get("text")
    language = row.get("language")
    prob = row.get("language_prob")
    if not isinstance(text, str) or not (language is None or isinstance(language, str)):
        raise invalid("asr", "draft text or language has the wrong type")
    if isinstance(prob, bool) or not isinstance(prob, int | float):
        raise invalid("asr", "draft language_prob is not a number")
    return Draft(text=text, language=language, language_prob=float(prob))


def guess(item: object) -> LanguageGuess:
    pair = listed_value(item, "lid")
    if len(pair) != 2 or not isinstance(pair[0], str):
        raise invalid("lid", f"guess {item!r} is not (code, prob)")
    return LanguageGuess(code=pair[0], prob=as_number(pair[1], "lid"))


def number_rows(result: Mapping[str, object], key: str, slot: str, width: int) -> list[list[float]]:
    rows = [
        [as_number(value, slot) for value in listed_value(row, slot)]
        for row in listed(result, key, slot)
    ]
    if any(len(row) != width for row in rows):
        raise invalid(slot, f"{key} rows must have {width} numbers")
    return rows


def listed(result: Mapping[str, object], key: str, slot: str) -> list[object]:
    if key not in result:
        raise invalid(slot, f"reply has no {key!r}")
    return listed_value(result[key], slot)


def listed_value(value: object, slot: str) -> list[object]:
    if not isinstance(value, list | tuple):
        raise invalid(slot, f"expected a list, got {type(value).__name__}")
    return list(value)


def number(result: Mapping[str, object], key: str, slot: str) -> float:
    return as_number(result.get(key), slot)


def as_number(value: object, slot: str) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise invalid(slot, f"expected a number, got {value!r}")
    return float(value)


def invalid(slot: str, detail: str) -> PermanentFailure:
    return PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"slot={slot} {detail}")
