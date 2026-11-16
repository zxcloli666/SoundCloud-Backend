from __future__ import annotations

from collections.abc import Mapping, Sequence

from worker.domain.outcome import PermanentFailure, Reason


def required_text(payload: Mapping[str, object], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a string")
    return value


def required_int(payload: Mapping[str, object], key: str) -> int:
    value = payload.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be an integer")
    return value


def required_mapping(payload: Mapping[str, object], key: str) -> Mapping[str, object]:
    value = payload.get(key)
    if not isinstance(value, Mapping):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be an object")
    return value


def required_list(payload: Mapping[str, object], key: str) -> Sequence[object]:
    value = payload.get(key)
    if not isinstance(value, list):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be an array")
    return value
