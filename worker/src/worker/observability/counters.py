from __future__ import annotations

from collections import defaultdict, deque
from collections.abc import Mapping

LATENCY_WINDOW = 512
MARK_WINDOW = 4096

LabelKey = tuple[tuple[str, str], ...]


def label_key(labels: Mapping[str, str]) -> LabelKey:
    return tuple(sorted((name, str(value)) for name, value in labels.items()))


class Counters:
    def __init__(self) -> None:
        self._counters: dict[str, dict[LabelKey, int]] = defaultdict(dict)
        self._gauges: dict[str, dict[LabelKey, float]] = defaultdict(dict)
        self._latency: dict[str, dict[LabelKey, deque[float]]] = defaultdict(dict)
        self._marks: dict[str, dict[LabelKey, deque[float]]] = defaultdict(dict)

    def inc(self, name: str, *, by: int = 1, **labels: str) -> int:
        key = label_key(labels)
        bucket = self._counters[name]
        bucket[key] = bucket.get(key, 0) + by
        return bucket[key]

    def gauge(self, name: str, value: float, **labels: str) -> None:
        self._gauges[name][label_key(labels)] = value

    def observe(self, name: str, value_ms: float, **labels: str) -> None:
        key = label_key(labels)
        window = self._latency[name].get(key)
        if window is None:
            window = deque(maxlen=LATENCY_WINDOW)
            self._latency[name][key] = window
        window.append(value_ms)

    def mark(self, name: str, at: float, **labels: str) -> None:
        key = label_key(labels)
        window = self._marks[name].get(key)
        if window is None:
            window = deque(maxlen=MARK_WINDOW)
            self._marks[name][key] = window
        window.append(at)

    def per_hour(self, name: str, now: float, window_s: float, **labels: str) -> float:
        marks = self._marks.get(name, {}).get(label_key(labels), ())
        recent = sum(1 for at in marks if now - at <= window_s)
        return round(recent * 3600.0 / window_s, 1)

    def latency(self, name: str, **labels: str) -> dict[str, float]:
        return quantiles(self._latency.get(name, {}).get(label_key(labels), deque()))

    def value(self, name: str, **labels: str) -> int:
        return self._counters.get(name, {}).get(label_key(labels), 0)

    def total(self, name: str) -> int:
        return sum(self._counters.get(name, {}).values())

    def snapshot(self) -> dict[str, dict[str, object]]:
        return {
            "counters": {name: _labelled(values) for name, values in self._counters.items()},
            "gauges": {name: _labelled(values) for name, values in self._gauges.items()},
            "latency_ms": {
                name: _labelled({key: quantiles(window) for key, window in windows.items()})
                for name, windows in self._latency.items()
            },
        }


def quantiles(samples: deque[float]) -> dict[str, float]:
    ordered = sorted(samples)
    if not ordered:
        return {"p50": 0.0, "p95": 0.0, "n": 0}
    return {
        "p50": ordered[int(0.50 * (len(ordered) - 1))],
        "p95": ordered[int(0.95 * (len(ordered) - 1))],
        "n": len(ordered),
    }


def _labelled(values: Mapping[LabelKey, object]) -> dict[str, object]:
    return {_label_text(key): value for key, value in values.items()}


def _label_text(key: LabelKey) -> str:
    return ",".join(f"{name}={value}" for name, value in key) if key else ""
