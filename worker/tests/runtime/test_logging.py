from __future__ import annotations

import io
import json
import logging
import os

from worker.observability.logging import JsonLog, StdlibBridge, install


def lines(stream: io.StringIO) -> list[dict[str, object]]:
    return [json.loads(line) for line in stream.getvalue().splitlines()]


def test_records_are_one_json_object_per_line_with_bound_fields() -> None:
    stream = io.StringIO()
    log = JsonLog(stream, wall=lambda: 1234.5678, lane="audio")
    log.info("job_finished", correlation="index_audio:1:1:1", duration_ms=12.5)
    log.bind(slot="muq").warning("slot_oom", recent=2)
    first, second = lines(stream)
    assert first == {
        "ts": 1234.568,
        "level": "info",
        "event": "job_finished",
        "pid": os.getpid(),
        "lane": "audio",
        "correlation": "index_audio:1:1:1",
        "duration_ms": 12.5,
    }
    assert second["slot"] == "muq" and second["lane"] == "audio" and second["level"] == "warning"


def test_level_threshold_filters_debug() -> None:
    stream = io.StringIO()
    log = JsonLog(stream, level="info")
    log.debug("hidden")
    log.error("shown")
    assert [record["event"] for record in lines(stream)] == ["shown"]
    assert log.bind(x=1).debug("still hidden") is None
    assert len(lines(stream)) == 1


def test_exception_carries_error_text_and_traceback() -> None:
    stream = io.StringIO()
    log = JsonLog(stream)
    try:
        raise ValueError("boom " * 200)
    except ValueError as error:
        log.exception("slot_call_failed", error, slot="asr")
    [record] = lines(stream)
    assert record["level"] == "error"
    assert str(record["error"]).startswith("ValueError: boom")
    assert len(str(record["error"])) <= 512
    assert "ValueError" in str(record["traceback"])


def test_non_json_values_are_stringified() -> None:
    stream = io.StringIO()
    JsonLog(stream).info("event", path=os.path, mapping={1: "x"})
    [record] = lines(stream)
    assert isinstance(record["path"], str)
    assert record["mapping"] == {"1": "x"}


def test_stdlib_bridge_routes_library_warnings() -> None:
    stream = io.StringIO()
    log = JsonLog(stream)
    logger = logging.getLogger("tests.runtime.bridge")
    logger.propagate = False
    handler = StdlibBridge(log)
    logger.addHandler(handler)
    try:
        logger.warning("model %s deprecated", "x")
        logger.info("ignored by threshold? no, handler has no level")
    finally:
        logger.removeHandler(handler)
    records = lines(stream)
    assert records[0] == {**records[0], "level": "warning", "event": "model x deprecated"}
    assert records[0]["logger"] == "tests.runtime.bridge"


def test_stdlib_bridge_keeps_extra_fields_and_the_traceback() -> None:
    stream = io.StringIO()
    logger = logging.getLogger("tests.runtime.bridge_extra")
    logger.propagate = False
    handler = StdlibBridge(JsonLog(stream))
    logger.addHandler(handler)
    try:
        logger.warning(
            "job_finished",
            extra={"lane": "audio", "status": "failed", "reason": "decode", "duration_ms": 12.5},
        )
        try:
            raise ValueError("boom from handler")
        except ValueError:
            logger.exception("handler_crashed", extra={"lane": "audio"})
    finally:
        logger.removeHandler(handler)
    finished, crashed = lines(stream)
    assert finished["lane"] == "audio"
    assert finished["status"] == "failed"
    assert finished["reason"] == "decode"
    assert finished["duration_ms"] == 12.5
    assert "msg" not in finished and "args" not in finished
    assert crashed["lane"] == "audio"
    assert crashed["error"] == "ValueError: boom from handler"
    assert isinstance(crashed["traceback"], str)
    assert "raise ValueError" in crashed["traceback"]


def test_install_replaces_root_handlers() -> None:
    root = logging.getLogger()
    before = list(root.handlers)
    stream = io.StringIO()
    try:
        install(JsonLog(stream), stdlib_level=logging.ERROR)
        logging.getLogger("lib").error("bad thing")
        logging.getLogger("lib").warning("filtered")
        assert [record["event"] for record in lines(stream)] == ["bad thing"]
    finally:
        for handler in list(root.handlers):
            root.removeHandler(handler)
        for handler in before:
            root.addHandler(handler)
