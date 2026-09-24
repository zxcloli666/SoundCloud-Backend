from __future__ import annotations

import math
from collections.abc import Mapping, Sequence

from worker.domain.outcome import PermanentFailure, Reason


def required_text(payload: Mapping[str, object], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a string")
    return value


def optional_text(payload: Mapping[str, object], key: str) -> str:
    value = payload.get(key)
    if value is None:
        return ""
    if not isinstance(value, str):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a string or null")
    return value


def optional_number(payload: Mapping[str, object], key: str) -> float | None:
    value = payload.get(key)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int | float) or not math.isfinite(value):
        raise PermanentFailure(Reason.INVALID_REQUEST, f"{key} must be a finite number or null")
    return float(value)


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


class UnreadableReply(ValueError):
    pass


def reply_text(value: object) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise UnreadableReply(f"expected a string, got {type(value).__name__}")
    return value.strip() or None


def reply_texts(value: object) -> tuple[str, ...]:
    if not isinstance(value, list):
        raise UnreadableReply(f"expected a list, got {type(value).__name__}")
    texts = [reply_text(item) for item in value]
    return tuple(text for text in texts if text is not None)


def reply_share(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise UnreadableReply(f"expected a number, got {type(value).__name__}")
    if not math.isfinite(value) or not 0.0 <= value <= 1.0:
        raise UnreadableReply(f"share {value} is outside [0, 1]")
    return float(value)


def reply_int(value: object) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise UnreadableReply(f"expected an integer, got {type(value).__name__}")
    return value
