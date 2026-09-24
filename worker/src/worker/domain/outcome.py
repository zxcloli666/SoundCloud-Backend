from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import StrEnum
from types import MappingProxyType

DETAIL_MAX_CHARS = 256


class Status(StrEnum):
    OK = "ok"
    EMPTY = "empty"
    MISSING = "missing"
    REJECTED = "rejected"
    FAILED = "failed"


class Reason(StrEnum):
    EMPTY_TEXT = "empty_text"
    EMPTY_REFERENCE_TEXT = "empty_reference_text"
    SILENT_AUDIO = "silent_audio"
    AUDIO_TOO_SHORT = "audio_too_short"
    EMPTY_VOCAB = "empty_vocab"
    TOO_FEW_USERS = "too_few_users"
    AUDIO_NOT_FOUND = "audio_not_found"
    AUDIO_FORBIDDEN = "audio_forbidden"
    OBJECT_NOT_FOUND = "object_not_found"
    LOW_CONFIDENCE = "low_confidence"
    TOO_FEW_LINES_PLACED = "too_few_lines_placed"
    NO_VOCAL_DETECTED = "no_vocal_detected"
    UNSUPPORTED_LANGUAGE = "unsupported_language"
    LYRICS_MISMATCH = "lyrics_mismatch"
    OUT_OF_ORDER = "out_of_order"
    PLACED_IN_SILENCE = "placed_in_silence"
    BELOW_BASELINE = "below_baseline"
    INVALID_REQUEST = "invalid_request"
    UNDECODABLE_AUDIO = "undecodable_audio"
    AUDIO_TOO_LONG = "audio_too_long"
    AUDIO_TOO_LARGE = "audio_too_large"
    TEXT_TOO_LONG_FOR_MODEL = "text_too_long_for_model"
    HASH_MISMATCH = "hash_mismatch"
    MODEL_OUTPUT_INVALID = "model_output_invalid"
    DEADLINE_EXCEEDED = "deadline_exceeded"
    ENGINE_CRASHED = "engine_crashed"
    OUT_OF_MEMORY = "out_of_memory"
    DOWNLOAD_FAILED = "download_failed"
    OBJECT_STORE_UNAVAILABLE = "object_store_unavailable"
    INTERNAL_ERROR = "internal_error"
    ENGINE_RESTARTED = "engine_restarted"
    WORKER_LOST = "worker_lost"
    PUBLIC_NODE_TIMEOUT = "public_node_timeout"


EMPTY_REASONS = frozenset(
    {
        Reason.EMPTY_TEXT,
        Reason.EMPTY_REFERENCE_TEXT,
        Reason.SILENT_AUDIO,
        Reason.AUDIO_TOO_SHORT,
        Reason.EMPTY_VOCAB,
        Reason.TOO_FEW_USERS,
    }
)
MISSING_REASONS = frozenset(
    {Reason.AUDIO_NOT_FOUND, Reason.AUDIO_FORBIDDEN, Reason.OBJECT_NOT_FOUND}
)
REJECTED_REASONS = frozenset(
    {
        Reason.LOW_CONFIDENCE,
        Reason.TOO_FEW_LINES_PLACED,
        Reason.NO_VOCAL_DETECTED,
        Reason.UNSUPPORTED_LANGUAGE,
        Reason.LYRICS_MISMATCH,
        Reason.OUT_OF_ORDER,
        Reason.PLACED_IN_SILENCE,
        Reason.BELOW_BASELINE,
    }
)
DETERMINISTIC_FAILURES = frozenset(
    {
        Reason.INVALID_REQUEST,
        Reason.UNDECODABLE_AUDIO,
        Reason.AUDIO_TOO_LONG,
        Reason.AUDIO_TOO_LARGE,
        Reason.TEXT_TOO_LONG_FOR_MODEL,
        Reason.HASH_MISMATCH,
        Reason.MODEL_OUTPUT_INVALID,
    }
)
TRANSIENT_FAILURES = frozenset(
    {
        Reason.DEADLINE_EXCEEDED,
        Reason.ENGINE_CRASHED,
        Reason.OUT_OF_MEMORY,
        Reason.DOWNLOAD_FAILED,
        Reason.OBJECT_STORE_UNAVAILABLE,
        Reason.INTERNAL_ERROR,
    }
)
REOPENABLE_FAILURES = frozenset(
    {Reason.ENGINE_RESTARTED, Reason.WORKER_LOST, Reason.PUBLIC_NODE_TIMEOUT}
)
FAILED_REASONS = DETERMINISTIC_FAILURES | TRANSIENT_FAILURES | REOPENABLE_FAILURES

REASONS_BY_STATUS: Mapping[Status, frozenset[Reason]] = MappingProxyType(
    {
        Status.OK: frozenset(),
        Status.EMPTY: EMPTY_REASONS,
        Status.MISSING: MISSING_REASONS,
        Status.REJECTED: REJECTED_REASONS,
        Status.FAILED: FAILED_REASONS,
    }
)


def status_of(reason: Reason) -> Status:
    for status, reasons in REASONS_BY_STATUS.items():
        if reason in reasons:
            return status
    raise ValueError(f"reason without status: {reason}")


@dataclass(frozen=True)
class Producer:
    worker_id: str
    build: str
    models: Mapping[str, str]
    sync_version: str | None

    def to_wire(self) -> dict[str, object]:
        return {
            "worker_id": self.worker_id,
            "build": self.build,
            "models": dict(self.models),
            "sync_version": self.sync_version,
        }


@dataclass(frozen=True)
class Outcome:
    status: Status
    reason: Reason | None = None
    detail: str | None = None
    fields: Mapping[str, object] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.status is Status.OK:
            if self.reason is not None:
                raise ValueError("ok outcome carries no reason")
            return
        if self.reason is None:
            raise ValueError(f"{self.status} outcome requires a reason")
        if self.reason not in REASONS_BY_STATUS[self.status]:
            raise ValueError(f"reason {self.reason} does not belong to status {self.status}")

    @classmethod
    def ok(cls, **fields: object) -> Outcome:
        return cls(Status.OK, fields=fields)

    @classmethod
    def of(cls, reason: Reason, detail: str | None = None, **fields: object) -> Outcome:
        return cls(status_of(reason), reason, detail, fields)

    @classmethod
    def failed(cls, reason: Reason, detail: str | None = None, **fields: object) -> Outcome:
        if reason not in FAILED_REASONS:
            raise ValueError(f"{reason} is not a failure reason")
        return cls(Status.FAILED, reason, detail, fields)

    def to_done(self, echo: Mapping[str, object], producer: Producer) -> dict[str, object]:
        done: dict[str, object] = dict(echo)
        done["status"] = self.status.value
        if self.reason is not None:
            done["reason"] = self.reason.value
        if self.detail:
            done["detail"] = self.detail[:DETAIL_MAX_CHARS]
        done["producer"] = producer.to_wire()
        done.update(self.fields)
        return done


class Failure(Exception):
    def __init__(self, reason: Reason, detail: str | None = None) -> None:
        super().__init__(detail or reason.value)
        self.reason = reason
        self.detail = detail


class PermanentFailure(Failure):
    def __init__(self, reason: Reason, detail: str | None = None) -> None:
        if reason in TRANSIENT_FAILURES or reason in REOPENABLE_FAILURES:
            raise ValueError(f"{reason} is not a permanent reason")
        super().__init__(reason, detail)

    def outcome(self, **fields: object) -> Outcome:
        return Outcome.of(self.reason, self.detail, **fields)


class TransientFailure(Failure):
    def __init__(self, reason: Reason, detail: str | None = None) -> None:
        if reason not in TRANSIENT_FAILURES:
            raise ValueError(f"{reason} is not a transient reason")
        super().__init__(reason, detail)

    def outcome(self, **fields: object) -> Outcome:
        return Outcome.failed(self.reason, self.detail, **fields)


class LeaseDropped(Exception):
    pass
