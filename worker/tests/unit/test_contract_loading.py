from __future__ import annotations

import ast
import json
import math
from pathlib import Path

import pytest
from nats.js import api

from tests.conftest import CONTRACT_PATH
from worker import contract as c
from worker.domain import outcome

SRC = Path(c.__file__).resolve().parent

PRODUCER = {"worker_id": "w", "build": "b", "models": {}, "sync_version": None}


def test_contract_loads_from_the_shipped_draft(contract: c.Contract) -> None:
    assert contract.version == 2
    assert set(contract.lanes) == {
        "audio",
        "lyrics",
        "transcribe",
        "encode",
        "collab",
        "taste",
        "ai",
    }
    assert contract.public_lanes == {"audio", "lyrics", "transcribe"}
    assert all(lane.durable == f"{name}-workers" for name, lane in contract.lanes.items())
    assert contract.dimensions == {
        "mert": 1024,
        "clap": 512,
        "lyrics": 1024,
        "mulan_text": 512,
        "collab": 128,
        "taste": 128,
    }
    assert contract.rpc.windows_s == {"resolve_artist": 20, "match_track": 10}
    assert contract.rpc.reply_header == "X-Reply-To"
    assert contract.max_message_bytes == 900_000


def test_wrong_version_is_rejected() -> None:
    raw = json.loads(CONTRACT_PATH.read_text())
    raw["version"] = 1
    with pytest.raises(c.ContractError, match="version"):
        c.parse(raw)


def test_lane_without_stream_is_rejected() -> None:
    raw = json.loads(CONTRACT_PATH.read_text())
    raw["lanes"]["audio"]["stream"] = "NOWHERE"
    with pytest.raises(c.ContractError, match="unknown stream"):
        c.parse(raw)


@pytest.mark.parametrize("name", ["audio", "lyrics", "transcribe", "encode", "collab", "taste"])
def test_window_formulas(contract: c.Contract, name: str) -> None:
    lane = contract.lane(name)
    delays = sum(lane.nak_delay(n) for n in range(1, lane.max_deliver))
    delivery = lane.bridge_ttl_s if lane.public else lane.deadline_s
    assert delivery is not None
    assert delivery >= lane.deadline_s
    assert lane.attempt_window_s == lane.max_deliver * delivery + delays
    stream = contract.streams[lane.stream]
    assert lane.quarantine_after_s == stream.max_age_s + lane.attempt_window_s
    if lane.public:
        assert lane.bridge_ttl_s == lane.deadline_s + 60
    else:
        assert lane.bridge_ttl_s is None


def test_design_examples(contract: c.Contract) -> None:
    transcribe = contract.lane("transcribe")
    assert [transcribe.nak_delay(n) for n in range(1, 5)] == [60, 120, 240, 480]
    assert transcribe.attempt_window_s == 5 * 960 + 60 + 120 + 240 + 480
    assert transcribe.quarantine_after_s == 86400 + 5700
    assert contract.lane("audio").attempt_window_s == 1650
    assert contract.lane("lyrics").attempt_window_s == 825
    assert contract.lane("taste").nak_delay(4) == 1800


def test_nak_delay_is_never_zero_and_capped(contract: c.Contract) -> None:
    for lane in contract.lanes.values():
        if lane.rpc:
            with pytest.raises(c.ContractError):
                lane.nak_delay(1)
            continue
        for n in range(1, 12):
            assert 0 < lane.nak_delay(n) <= lane.nak_cap_s
        assert lane.nak_delay(0) == lane.nak_base_s


def test_last_delivery(contract: c.Contract) -> None:
    lane = contract.lane("audio")
    assert not lane.is_last_delivery(4)
    assert lane.is_last_delivery(5)
    assert lane.is_last_delivery(6)
    assert lane.is_last_delivery(1, max_deliver=1)
    assert not lane.is_last_delivery(99, max_deliver=-1)


def test_correlation_and_done_msg_id(contract: c.Contract) -> None:
    lane = contract.lane("audio")
    key = lane.correlation_key({"sc_track_id": "98765", "upload_generation": 3, "attempt": 1})
    assert key == "index_audio:98765:3:1"
    assert lane.done_msg_id(key, 77, "ok") == "done.audio:index_audio:98765:3:1:77:ok"
    assert lane.done_msg_id_template == "done.{lane}:{correlation}:{task_seq}:{status}"
    assert lane.correlation_separator == ":"
    with pytest.raises(c.ContractError, match="no done"):
        contract.lane("ai").done_msg_id("ai", 1, "ok")
    assert (
        contract.lane("encode").correlation_key({"model": "mulan", "hash": "ab"})
        == "encode:mulan:ab"
    )
    assert (
        contract.lane("collab").correlation_key({"object": "collab-input-1"})
        == "train_collab:collab-input-1"
    )
    with pytest.raises(c.ContractError, match="attempt"):
        lane.correlation_key({"sc_track_id": "1", "upload_generation": 1})


def test_done_msg_id_and_correlation_follow_the_exported_templates() -> None:
    raw = json.loads(CONTRACT_PATH.read_text())
    raw["lanes"]["audio"]["done_msg_id_template"] = "{status}|{task_seq}|{correlation}|{lane}"
    raw["lanes"]["audio"]["correlation_separator"] = "/"
    lane = c.parse(raw).lane("audio")
    key = lane.correlation_key({"sc_track_id": "7", "upload_generation": 2, "attempt": 1})
    assert key == "index_audio/7/2/1"
    assert lane.done_msg_id(key, 5, "ok") == "ok|5|index_audio/7/2/1|audio"


def test_a_non_canonical_track_id_is_refused_by_every_schema(contract: c.Contract) -> None:
    for name, schema in contract.schemas.items():
        if "sc_track_id" not in schema.get("properties", {}):
            continue
        for raw in ("0", "042", "99999999999999999999"):
            errors = contract.validate(name, {"sc_track_id": raw})
            assert any(error.startswith("sc_track_id:") for error in errors), (name, raw)
        errors = contract.validate(name, {"sc_track_id": "98765"})
        assert not any(error.startswith("sc_track_id:") for error in errors), name


def test_echo_fields_rename_collab_object(contract: c.Contract) -> None:
    assert contract.lane("collab").echo_fields({"object": "o", "dim": 128}) == {"input_object": "o"}
    assert contract.lane("transcribe").echo_fields(
        {"sc_track_id": "1", "upload_generation": 2, "attempt": 3, "mode": "align", "x": 1}
    ) == {"sc_track_id": "1", "upload_generation": 2, "attempt": 3, "mode": "align"}


def test_reopenable_lanes_carry_attempt_request_id_or_object(contract: c.Contract) -> None:
    reopenable_class = set(contract.reason_classes["reopenable"])
    for lane in contract.lanes.values():
        assert lane.reopenable <= reopenable_class
        if lane.reopenable:
            assert {"attempt", "request_id", "object"} & set(lane.correlation)
    assert contract.lane("audio").reopenable == {
        "engine_restarted",
        "worker_lost",
        "public_node_timeout",
    }
    assert contract.lane("lyrics").reopenable == {"engine_restarted", "worker_lost"}
    assert contract.lane("encode").reopenable == frozenset()
    assert contract.lane("ai").reopenable == frozenset()


def test_reasons_match_the_domain_enum(contract: c.Contract) -> None:
    listed = {reason for reasons in contract.reasons.values() for reason in reasons}
    assert listed == {reason.value for reason in outcome.Reason}
    for status, reasons in contract.reasons.items():
        expected = {r.value for r in outcome.REASONS_BY_STATUS[outcome.Status(status)]}
        assert set(reasons) == expected, status
    classes = contract.reason_classes
    assert set(classes["deterministic"]) == {r.value for r in outcome.DETERMINISTIC_FAILURES}
    assert set(classes["transient"]) == {r.value for r in outcome.TRANSIENT_FAILURES}
    assert set(classes["reopenable"]) == {r.value for r in outcome.REOPENABLE_FAILURES}


def test_result_limits_fit_the_server(contract: c.Contract) -> None:
    for lane in contract.lanes.values():
        assert lane.result_max_bytes <= contract.max_message_bytes
    assert contract.lane("transcribe").result_max_bytes == 512 * 1024


def test_pipeline_done_window_covers_the_longest_attempt_window(contract: c.Contract) -> None:
    done = contract.streams["PIPELINE_DONE"]
    longest = max(lane.attempt_window_s or 0 for lane in contract.lanes.values())
    assert done.duplicate_window_s == math.ceil(longest / 3600) * 3600 == 43200
    assert done.duplicate_window_s <= done.max_age_s == 72 * 3600
    assert done.discard == "old"
    assert done.max_bytes == 24 * 1024**3
    assert contract.streams["INDEX_AUDIO"].discard == "new"
    assert contract.streams["WORKER_INVALID"].retention == "limits"


def test_every_lane_has_schemas_for_request_and_done(contract: c.Contract) -> None:
    for lane in contract.lanes.values():
        if lane.rpc:
            for method in contract.rpc.windows_s:
                assert f"ai.rpc.{method}" in contract.schemas
                assert f"ai.rpc.{method}.reply" in contract.schemas
            continue
        assert lane.filter_subject in contract.schemas
        assert lane.done_subject in contract.schemas
        done_schema = contract.schemas[lane.done_subject]
        assert {"status", "producer"} <= set(done_schema["required"])
        assert set(lane.echo.values()) <= set(done_schema["required"])


def test_subjects_and_headers(contract: c.Contract) -> None:
    assert contract.invalid_subject("audio") == "worker.invalid.audio"
    assert contract.status_subject("gpu-main") == "worker.status.gpu-main"
    assert contract.health_subject("gpu-main") == "worker.health.gpu-main"
    assert contract.headers == c.HeaderNames(
        msg_id="Nats-Msg-Id",
        reply_to="X-Reply-To",
        deadline="X-Deadline",
        worker_id="X-Worker-Id",
        worker_build="X-Worker-Build",
        deliveries="X-Deliveries",
        public_node="X-Public-Node",
    )
    assert contract.headers.msg_id == api.Header.MSG_ID.value
    assert contract.rpc.reply_header == contract.headers.reply_to
    assert contract.rpc.deadline_header == contract.headers.deadline


def test_a_contract_without_a_header_role_is_refused() -> None:
    raw = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    del raw["headers"]["deliveries"]
    with pytest.raises(c.ContractError, match="deliveries"):
        c.parse(raw)


def test_a_worker_never_publishes_a_reason_the_bus_sets(contract: c.Contract) -> None:
    set_by_bus = set().union(
        *(lane.reasons - lane.worker_reasons for lane in contract.lanes.values())
    )
    assert set_by_bus == {"worker_lost", "public_node_timeout"}
    for lane in contract.lanes.values():
        assert lane.worker_reasons <= lane.reasons, lane.name
    published = reasons_named_in_worker_code()
    assert published
    assert not published & set_by_bus
    every_worker_reason = set().union(*(lane.worker_reasons for lane in contract.lanes.values()))
    assert published <= every_worker_reason


def reasons_named_in_worker_code() -> set[str]:
    named: set[str] = set()
    for path in SRC.rglob("*.py"):
        if path == SRC / "domain" / "outcome.py":
            continue
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            if not isinstance(node, ast.Attribute) or not isinstance(node.value, ast.Name):
                continue
            if node.value.id == "Reason":
                named.add(outcome.Reason[node.attr].value)
    return named


def test_request_validation(contract: c.Contract) -> None:
    good = {"sc_track_id": "98765", "s3_url": "https://s3/x", "upload_generation": 3, "attempt": 1}
    assert contract.validate("index.audio.new", good) == []
    assert contract.validate("index.audio.new", {**good, "extra": 1}) == []
    errors = contract.validate("index.audio.new", {**good, "sc_track_id": "abc", "attempt": 0})
    assert any("sc_track_id" in e for e in errors) and any("attempt" in e for e in errors)
    assert contract.validate("encode.text.new", {"model": "bge", "text": "", "hash": "zz"})
    assert (
        contract.validate(
            "transcribe.audio.new",
            {
                "sc_track_id": "1",
                "upload_generation": 1,
                "attempt": 1,
                "audio_url": "https://a",
                "reference_text": "la la",
                "reference_lines_total": 2,
                "language": None,
                "mode": "align",
            },
        )
        == []
    )
    assert contract.validate("train.collab.new", {"object": "o", "dataset_version": 1, "dim": 128})
    with pytest.raises(c.ContractError):
        contract.validate("nope", {})


def test_done_validation_ties_reason_to_status(contract: c.Contract) -> None:
    base = {"sc_track_id": "1", "upload_generation": 1, "attempt": 1, "producer": PRODUCER}
    ok = {**base, "status": "ok", "mert": [0.0] * 1024, "clap": [0.0] * 512, "fingerprint": None}
    assert contract.validate("done.index_audio", ok) == []
    assert contract.validate("done.index_audio", {**ok, "reason": "silent_audio"})
    assert contract.validate("done.index_audio", {**base, "status": "ok"})
    assert contract.validate("done.index_audio", {**base, "status": "empty"})
    assert contract.validate(
        "done.index_audio", {**base, "status": "empty", "reason": "audio_not_found"}
    )
    assert (
        contract.validate("done.index_audio", {**base, "status": "empty", "reason": "silent_audio"})
        == []
    )
    encode = {"model": "lyrics", "hash": "a" * 64, "producer": PRODUCER, "vector": None}
    assert (
        contract.validate("done.encode", {**encode, "status": "empty", "reason": "empty_text"})
        == []
    )
    assert contract.validate(
        "done.encode", {**encode, "status": "empty", "reason": "audio_not_found"}
    )
    collab = {
        "input_object": "o",
        "producer": PRODUCER,
        "trained": False,
        "dim": 128,
        "points_count": 0,
    }
    assert (
        contract.validate(
            "done.train_collab", {**collab, "status": "empty", "reason": "empty_vocab"}
        )
        == []
    )
    assert contract.validate(
        "done.train_collab", {**collab, "status": "empty", "reason": "audio_not_found"}
    )
    assert contract.validate(
        "done.train_collab", {**collab, "status": "missing", "reason": "silent_audio"}
    )


def test_every_done_schema_ties_each_reason_to_its_own_status(contract: c.Contract) -> None:
    for lane in contract.lanes.values():
        if lane.done_subject is None:
            continue
        accepted: set[str] = set()
        for status, allowed in accepted_reasons(contract, lane.done_subject).items():
            assert allowed <= set(contract.reasons[status]), (lane.name, status)
            accepted |= allowed
        assert accepted == lane.reasons, lane.name


def accepted_reasons(contract: c.Contract, done_subject: str) -> dict[str, set[str]]:
    accepted = {}
    for rule in contract.schemas[done_subject]["allOf"]:
        status = rule["if"]["properties"]["status"]["const"]
        if status == "ok":
            continue
        then = rule["then"]
        accepted[status] = set() if then is False else set(then["properties"]["reason"]["enum"])
    return accepted


def test_a_done_refuses_a_reason_foreign_to_its_lane(contract: c.Contract) -> None:
    audio = {
        "sc_track_id": "1",
        "upload_generation": 1,
        "attempt": 1,
        "producer": PRODUCER,
    }
    assert contract.validate(
        "done.index_audio", {**audio, "status": "rejected", "reason": "low_confidence"}
    )
    assert contract.validate(
        "done.index_audio", {**audio, "status": "empty", "reason": "empty_vocab"}
    )
    silent = {**audio, "status": "empty", "reason": "silent_audio"}
    assert contract.validate("done.index_audio", silent) == []
    encode = {"model": "mulan", "hash": "a" * 64, "producer": PRODUCER, "vector": None}
    assert contract.validate(
        "done.encode", {**encode, "status": "rejected", "reason": "lyrics_mismatch"}
    )


def test_failed_reopenable_reasons_are_limited_to_the_lane(contract: c.Contract) -> None:
    reopenable = set(contract.reason_classes["reopenable"])
    for lane in contract.lanes.values():
        if lane.done_subject is None:
            continue
        allowed = accepted_reasons(contract, lane.done_subject)["failed"]
        assert allowed & reopenable == lane.reopenable, lane.name
        assert {"invalid_request", "deadline_exceeded", "internal_error"} <= allowed
    lyrics = {"sc_track_id": "1", "request_id": "r", "producer": PRODUCER, "status": "failed"}
    assert contract.validate("done.embed_lyrics", {**lyrics, "reason": "public_node_timeout"})
    assert contract.validate("done.embed_lyrics", {**lyrics, "reason": "worker_lost"}) == []
    encode = {"model": "mulan", "hash": "a" * 64, "producer": PRODUCER, "vector": None}
    failed = {**encode, "status": "failed"}
    assert contract.validate("done.encode", {**failed, "reason": "engine_restarted"})
    assert contract.validate("done.encode", {**failed, "reason": "deadline_exceeded"}) == []


def test_done_embed_lyrics_requires_language_when_empty(contract: c.Contract) -> None:
    base = {"sc_track_id": "1", "request_id": "r", "producer": PRODUCER}
    empty = {**base, "status": "empty", "reason": "empty_text"}
    assert contract.validate("done.embed_lyrics", empty)
    assert contract.validate("done.embed_lyrics", {**empty, "language": None}) == []
    assert contract.validate("done.embed_lyrics", {**empty, "language": "ru"}) == []


def test_text_limits_count_utf8_bytes(contract: c.Contract) -> None:
    encode = {"model": "lyrics", "hash": "a" * 64}
    assert contract.validate("encode.text.new", {**encode, "text": "я" * 256}) == []
    too_long = contract.validate("encode.text.new", {**encode, "text": "я" * 512})
    assert too_long == ["text: 1024 UTF-8 bytes is longer than 512"]
    assert contract.validate("encode.text.new", {**encode, "text": "a" * 512}) == []
    assert contract.validate("encode.text.new", {**encode, "text": "\ud800"})
    lyrics = {"sc_track_id": "1", "request_id": "r"}
    assert contract.validate("embed.lyrics.new", {**lyrics, "text": "ж" * 8000}) == []
    assert contract.validate("embed.lyrics.new", {**lyrics, "text": "ж" * 8001})
    schema = contract.schemas["transcribe.audio.new"]["properties"]
    assert schema["reference_text"][c.MAX_UTF8_BYTES] == 16000


def test_done_encode_vector_matches_model(contract: c.Contract) -> None:
    base = {"hash": "a" * 64, "producer": PRODUCER, "status": "ok"}
    assert contract.validate("done.encode", {**base, "model": "mulan", "vector": [0.0] * 512}) == []
    assert contract.validate("done.encode", {**base, "model": "mulan", "vector": [0.0] * 1024})
    assert (
        contract.validate("done.encode", {**base, "model": "lyrics", "vector": [0.0] * 1024}) == []
    )
    empty = {**base, "model": "lyrics", "status": "empty", "reason": "empty_text", "vector": None}
    assert contract.validate("done.encode", empty) == []
    assert contract.validate("done.encode", {**empty, "vector": [0.0] * 1024})


def test_done_transcribe_requires_metrics_when_rejected(contract: c.Contract) -> None:
    base = {
        "sc_track_id": "1",
        "upload_generation": 1,
        "attempt": 1,
        "mode": "align",
        "producer": {**PRODUCER, "sync_version": "s4.aaaaaaaa.bbbbbbbb.cccccccc"},
        "sync_version": "s4.aaaaaaaa.bbbbbbbb.cccccccc",
    }
    rejected = {**base, "status": "rejected", "reason": "lyrics_mismatch"}
    assert contract.validate("done.transcribe", rejected)
    metrics = {
        "confidence": 0.3,
        "placed_share": 0.5,
        "aligned_share": 0.4,
        "lines_total": 40,
        "lines_unplaced": 20,
        "language": "ru",
    }
    assert contract.validate("done.transcribe", {**rejected, **metrics}) == []
    ok = {**base, "status": "ok", **metrics, "synced_lrc": "[00:01.00] la"}
    assert contract.validate("done.transcribe", ok) == []
    words = [
        {
            "line": 0,
            "text": "la",
            "start_ms": 1000,
            "end_ms": 1200,
            "interpolated": False,
            "unplaced": False,
        }
    ]
    assert contract.validate("done.transcribe", {**ok, "words": words}) == []
    assert contract.validate("done.transcribe", {**ok, "language": "rus"})
    assert contract.validate("done.transcribe", {**ok, "producer": PRODUCER})
    no_sync = {key: value for key, value in PRODUCER.items() if key != "sync_version"}
    assert contract.validate("done.transcribe", {**ok, "producer": no_sync})


def test_rpc_reply_schemas(contract: c.Contract) -> None:
    resolve = {
        "ok": True,
        "data": {
            "primary_artist": "Kendrick Lamar",
            "featured": ["SZA"],
            "producers": [],
            "remixers": [],
            "album": None,
            "confidence": 0.93,
            "source": "llm",
        },
    }
    assert contract.validate("ai.rpc.resolve_artist.reply", resolve) == []
    assert contract.validate("ai.rpc.resolve_artist.reply", {"ok": False, "error": "expired"}) == []
    assert contract.validate("ai.rpc.resolve_artist.reply", {"ok": False, "error": "boom"})
    match = {"ok": True, "data": {"match_id": None, "confidence": 0.4, "source": "deterministic"}}
    assert contract.validate("ai.rpc.match_track.reply", match) == []
    widest = {**match, "data": {**match["data"], "match_id": 2**32 - 1}}
    assert contract.validate("ai.rpc.match_track.reply", widest) == []
    beyond = {**match, "data": {**match["data"], "match_id": 2**32}}
    assert contract.validate("ai.rpc.match_track.reply", beyond)
    request = {
        "target": {"artist": "a", "title": "t"},
        "candidates": [{"id": 1, "artist": "a", "title": "t"}],
    }
    assert contract.validate("ai.rpc.match_track", request) == []
    assert contract.validate("ai.rpc.match_track", {**request, "candidates": []})
