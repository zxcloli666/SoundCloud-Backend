from __future__ import annotations

from worker.observability.counters import Counters, quantiles


def test_counters_with_labels() -> None:
    counters = Counters()
    assert counters.inc("ack_failures_total", lane="audio") == 1
    assert counters.inc("ack_failures_total", lane="audio", by=2) == 3
    counters.inc("ack_failures_total", lane="lyrics")
    counters.inc("lease_lost_total")
    assert counters.value("ack_failures_total", lane="audio") == 3
    assert counters.value("ack_failures_total", lane="nope") == 0
    assert counters.total("ack_failures_total") == 4
    assert counters.value("lease_lost_total") == 1


def test_gauges_and_latency_snapshot() -> None:
    counters = Counters()
    counters.gauge("outbox_pending", 12)
    counters.gauge("slot_reserved_gap_mib", 700, slot="muq")
    for value in range(1, 101):
        counters.observe("slot_call_ms", float(value), slot="muq")
    counters.inc("done_total", lane="audio", status="ok")
    snapshot = counters.snapshot()
    assert snapshot["counters"]["done_total"] == {"lane=audio,status=ok": 1}
    assert snapshot["gauges"]["outbox_pending"] == {"": 12}
    assert snapshot["gauges"]["slot_reserved_gap_mib"] == {"slot=muq": 700}
    latency = snapshot["latency_ms"]["slot_call_ms"]["slot=muq"]
    assert latency["p50"] == 50.0 and latency["p95"] == 95.0 and latency["n"] == 100


def test_quantiles_of_empty_window() -> None:
    from collections import deque

    assert quantiles(deque()) == {"p50": 0.0, "p95": 0.0, "n": 0}
    assert quantiles(deque([7.0])) == {"p50": 7.0, "p95": 7.0, "n": 1}
