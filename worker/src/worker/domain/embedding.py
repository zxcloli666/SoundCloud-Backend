from __future__ import annotations

import logging
from collections.abc import Mapping

import numpy as np

from worker.domain.outcome import (
    LeaseDropped,
    Outcome,
    PermanentFailure,
    Reason,
    TransientFailure,
)
from worker.domain.ports import EngineUnavailable, Float32Array
from worker.observability.counters import Counters

MIN_NORM = 0.99
MAX_NORM = 1.01

log = logging.getLogger(__name__)


def pooled(rows: Float32Array, dim: int, slot: str, counters: Counters) -> Float32Array:
    checked = checked_rows(rows, dim, slot, counters)
    mean = checked.mean(axis=0, dtype=np.float64)
    norm = float(np.linalg.norm(mean))
    if not np.isfinite(norm) or norm == 0.0:
        raise invalid_output(slot, "pooled vector has zero norm", counters)
    return (mean / norm).astype(np.float32)


def single(rows: Float32Array, dim: int, slot: str, counters: Counters) -> Float32Array:
    checked = checked_rows(rows, dim, slot, counters)
    if checked.shape[0] != 1:
        raise invalid_output(slot, f"rows={checked.shape[0]} expected 1", counters)
    return np.ascontiguousarray(checked[0], dtype=np.float32)


def checked_rows(rows: Float32Array, dim: int, slot: str, counters: Counters) -> Float32Array:
    if rows.ndim != 2 or rows.shape[0] == 0 or rows.shape[1] != dim:
        raise invalid_output(slot, f"shape={tuple(rows.shape)} dim={dim}", counters)
    if not np.all(np.isfinite(rows)):
        raise invalid_output(slot, "non-finite values", counters)
    norms = np.linalg.norm(rows.astype(np.float64), axis=1)
    if np.any(norms < MIN_NORM) or np.any(norms > MAX_NORM):
        raise invalid_output(slot, f"norms={norms.min():.4f}..{norms.max():.4f}", counters)
    return rows


def invalid_output(slot: str, detail: str, counters: Counters) -> PermanentFailure:
    counters.inc("model_output_invalid_total", slot=slot)
    log.error("model output invalid", extra={"slot": slot, "detail": detail})
    return PermanentFailure(Reason.MODEL_OUTPUT_INVALID, f"slot={slot} {detail}")


def text_field(request: Mapping[str, object], key: str) -> str:
    value = request.get(key)
    if not isinstance(value, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a string")
    return value


def optional_text_field(request: Mapping[str, object], key: str) -> str | None:
    value = request.get(key)
    if value is None or isinstance(value, str):
        return value
    raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a string or null")


def outcome_of_error(error: Exception, lane: str, counters: Counters, **fields: object) -> Outcome:
    if isinstance(error, LeaseDropped):
        raise error
    if isinstance(error, PermanentFailure | TransientFailure):
        return error.outcome(**fields)
    if isinstance(error, EngineUnavailable):
        counters.inc("lane_engine_unavailable_total", lane=lane, slot=error.slot)
        log.warning(
            "engine unavailable", extra={"lane": lane, "slot": error.slot, "state": error.state}
        )
        detail = f"slot={error.slot} state={error.state}"
        return TransientFailure(Reason.ENGINE_CRASHED, detail).outcome(**fields)
    counters.inc("lane_internal_errors_total", lane=lane, error=type(error).__name__)
    log.error("lane failed unexpectedly", extra={"lane": lane}, exc_info=error)
    detail = f"{type(error).__name__}: {error}"
    return TransientFailure(Reason.INTERNAL_ERROR, detail).outcome(**fields)
