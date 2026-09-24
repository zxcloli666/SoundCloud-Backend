from __future__ import annotations

import pytest

from worker.domain import outcome as o

PRODUCER = o.Producer(
    "gpu-main", "2026.09.1", {"mert": "OpenMuQ/MuQ-large-msd-iter@0562a578"}, None
)


def test_every_reason_has_exactly_one_status() -> None:
    seen: dict[o.Reason, o.Status] = {}
    for status, reasons in o.REASONS_BY_STATUS.items():
        for reason in reasons:
            assert reason not in seen
            seen[reason] = status
    assert set(seen) == set(o.Reason)
    assert o.status_of(o.Reason.AUDIO_NOT_FOUND) is o.Status.MISSING
    assert o.status_of(o.Reason.ENGINE_RESTARTED) is o.Status.FAILED


def test_failure_classes_partition_failed() -> None:
    assert o.DETERMINISTIC_FAILURES.isdisjoint(o.TRANSIENT_FAILURES)
    assert o.TRANSIENT_FAILURES.isdisjoint(o.REOPENABLE_FAILURES)
    assert (
        o.DETERMINISTIC_FAILURES | o.TRANSIENT_FAILURES | o.REOPENABLE_FAILURES == o.FAILED_REASONS
    )


def test_ok_outcome_to_done() -> None:
    done = o.Outcome.ok(mert=[0.1], clap=[0.2], fingerprint=None).to_done(
        {"sc_track_id": "1", "upload_generation": 2, "attempt": 3}, PRODUCER
    )
    assert done["status"] == "ok"
    assert "reason" not in done and "detail" not in done
    assert done["producer"] == {
        "worker_id": "gpu-main",
        "build": "2026.09.1",
        "models": {"mert": "OpenMuQ/MuQ-large-msd-iter@0562a578"},
        "sync_version": None,
    }
    assert done["mert"] == [0.1] and done["fingerprint"] is None
    assert done["sc_track_id"] == "1"


def test_reason_outcome_carries_status_reason_and_truncated_detail() -> None:
    long_detail = "x" * 300
    done = o.Outcome.of(o.Reason.LYRICS_MISMATCH, long_detail, confidence=0.2).to_done({}, PRODUCER)
    assert done["status"] == "rejected"
    assert done["reason"] == "lyrics_mismatch"
    assert len(done["detail"]) == o.DETAIL_MAX_CHARS
    assert done["confidence"] == 0.2


def test_outcome_constructor_guards() -> None:
    with pytest.raises(ValueError):
        o.Outcome(o.Status.OK, o.Reason.EMPTY_TEXT)
    with pytest.raises(ValueError):
        o.Outcome(o.Status.EMPTY)
    with pytest.raises(ValueError):
        o.Outcome(o.Status.EMPTY, o.Reason.AUDIO_NOT_FOUND)
    with pytest.raises(ValueError):
        o.Outcome.failed(o.Reason.SILENT_AUDIO)


def test_permanent_failure_becomes_a_terminal_outcome() -> None:
    failure = o.PermanentFailure(o.Reason.OBJECT_NOT_FOUND, "bucket/x")
    result = failure.outcome(input_object="x")
    assert result.status is o.Status.MISSING
    assert result.reason is o.Reason.OBJECT_NOT_FOUND
    assert result.fields == {"input_object": "x"}
    with pytest.raises(ValueError):
        o.PermanentFailure(o.Reason.DEADLINE_EXCEEDED)
    with pytest.raises(ValueError):
        o.PermanentFailure(o.Reason.WORKER_LOST)


def test_transient_failure_becomes_failed_on_last_delivery() -> None:
    failure = o.TransientFailure(o.Reason.DOWNLOAD_FAILED, "503")
    assert str(failure) == "503"
    result = failure.outcome()
    assert result.status is o.Status.FAILED and result.reason is o.Reason.DOWNLOAD_FAILED
    with pytest.raises(ValueError):
        o.TransientFailure(o.Reason.INVALID_REQUEST)
    with pytest.raises(ValueError):
        o.TransientFailure(o.Reason.ENGINE_RESTARTED)
