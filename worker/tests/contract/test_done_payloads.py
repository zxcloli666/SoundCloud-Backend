from __future__ import annotations

import re

import orjson
import pytest

from tests.contract import samples
from worker.bus.outbox import encode
from worker.contract import Contract
from worker.domain.outcome import (
    DETAIL_MAX_CHARS,
    REOPENABLE_FAILURES,
    Outcome,
    Reason,
)

TASK_LANES = tuple(samples.REQUESTS)
DESIGN_PAIRS = [
    (lane, reason) for lane, reasons in samples.DESIGN_REASONS.items() for reason in sorted(reasons)
]
DESIGN_CORRELATION = {
    "audio": "index_audio:98765:3:1",
    "lyrics": "embed_lyrics:98765:lyr:98765:4",
    "transcribe": "transcribe:98765:3:2",
    "encode": f"encode:lyrics:{samples.REQUESTS['encode']['hash']}",
    "collab": "train_collab:collab-input-7c1d",
    "taste": "train_taste:taste-input-9f2e",
}
LRC_LINE = "[59:59.99]а\n"
REFERENCE_TEXT_BYTES = 16000


@pytest.mark.parametrize("lane", TASK_LANES)
def test_request_sample_is_valid(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    assert contract.validate(spec.filter_subject, samples.REQUESTS[lane]) == []


@pytest.mark.parametrize("lane", TASK_LANES)
def test_ok_done_is_valid(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    done = samples.done_payload(spec, None)
    assert contract.validate(str(spec.done_subject), done) == []
    assert "reason" not in done


@pytest.mark.parametrize(("lane", "reason"), DESIGN_PAIRS, ids=lambda value: str(value))
def test_every_design_reason_of_a_lane_is_a_valid_done(
    contract: Contract, lane: str, reason: Reason
) -> None:
    spec = contract.lane(lane)
    done = samples.done_payload(spec, reason)
    assert contract.validate(str(spec.done_subject), done) == []
    assert (done["status"], done["reason"]) == (Outcome.of(reason).status.value, reason.value)


@pytest.mark.parametrize("lane", TASK_LANES)
def test_worker_publishes_only_its_own_reopenable_reason(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    published = samples.DESIGN_REASONS[lane] & REOPENABLE_FAILURES
    assert {reason.value for reason in published} <= spec.reopenable
    assert (Reason.ENGINE_RESTARTED in published) == (Reason.ENGINE_RESTARTED in spec.reopenable)


@pytest.mark.parametrize("lane", TASK_LANES)
def test_long_detail_is_cut_to_the_wire_limit(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    fields = samples.fields_for(lane, Outcome.of(Reason.INTERNAL_ERROR).status)
    outcome = Outcome.of(Reason.INTERNAL_ERROR, "я" * (DETAIL_MAX_CHARS * 4), **fields)
    done = outcome.to_done(spec.echo_fields(samples.REQUESTS[lane]), samples.producer())
    if lane == "transcribe":
        done["producer"] = samples.producer(samples.SYNC_VERSION).to_wire()
    assert contract.validate(str(spec.done_subject), done) == []
    assert len(str(done["detail"])) == DETAIL_MAX_CHARS


@pytest.mark.parametrize("lane", TASK_LANES)
def test_correlation_and_done_msg_id_follow_the_design(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    correlation = spec.correlation_key(samples.REQUESTS[lane])
    assert correlation == DESIGN_CORRELATION[lane]
    first = spec.done_msg_id(correlation, 41, "failed")
    assert re.fullmatch(rf"done\.{lane}:{re.escape(correlation)}:41:failed", first)
    assert spec.done_msg_id(correlation, 41, "failed") == first
    assert spec.done_msg_id(correlation, 42, "failed") != first
    assert spec.done_msg_id(correlation, 41, "ok") != first


@pytest.mark.parametrize("lane", TASK_LANES)
def test_largest_ok_done_fits_the_lane_limit(contract: Contract, lane: str) -> None:
    spec = contract.lane(lane)
    done = largest_ok_done(contract, lane)
    size = len(encode(done))
    assert size <= spec.result_max_bytes, f"{lane}: {size} > {spec.result_max_bytes}"


def largest_ok_done(contract: Contract, lane: str) -> dict[str, object]:
    spec = contract.lane(lane)
    done = samples.done_payload(spec, None)
    for field, dim in (("mert", 1024), ("clap", 512), ("vec", 1024), ("vector", 1024)):
        if field in done:
            done[field] = samples.float32_vector(dim)
    if lane == "transcribe":
        lines = REFERENCE_TEXT_BYTES // len("а\n".encode())
        done["synced_lrc"] = LRC_LINE * lines
        done["lines_total"] = lines
        del done["words"]
    return done


@pytest.mark.parametrize("method", ["ai.rpc.resolve_artist", "ai.rpc.match_track"])
def test_rpc_request_samples_are_valid(contract: Contract, method: str) -> None:
    assert contract.validate(method, samples.RPC_REQUESTS[method]) == []
    assert method.startswith(contract.lane("ai").filter_subject.removesuffix(">"))


@pytest.mark.parametrize("schema", sorted(samples.RPC_REPLIES))
def test_rpc_replies_are_valid(contract: Contract, schema: str) -> None:
    ai = contract.lane("ai")
    for reply in samples.RPC_REPLIES[schema]:
        assert contract.validate(schema, reply) == []
        assert len(orjson.dumps(reply)) <= ai.result_max_bytes
    for error in samples.RPC_ERRORS:
        assert contract.validate(schema, {"ok": False, "error": error}) == []
    assert contract.validate(schema, {"ok": False, "error": "timeout"}) != []
    assert contract.validate(schema, {"ok": True}) != []


def test_rpc_windows_bound_the_ai_lane(contract: Contract) -> None:
    ai = contract.lane("ai")
    assert contract.rpc.windows_s == {"resolve_artist": 20, "match_track": 10}
    assert ai.deadline_s <= max(contract.rpc.windows_s.values())
    assert ai.done_subject is None
    assert ai.nak_base_s is None and ai.reopenable == frozenset()
