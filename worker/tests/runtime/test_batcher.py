from __future__ import annotations

import asyncio
import os
import time
from collections.abc import Callable
from dataclasses import dataclass, field

import numpy as np
import pytest

from tests.runtime.support import quiet_log
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.batcher import Batcher, prepare
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    DeadlineExceeded,
    EngineCrashed,
    EngineError,
    EngineKilled,
    SlotUnavailable,
)
from worker.runtime.protocol import Call, ErrorKind, Reply

Behaviour = Callable[[Call, dict[str, np.ndarray]], Reply]


def echo(call: Call, arrays: dict[str, np.ndarray]) -> Reply:
    outputs = {key: value * 2 for key, value in arrays.items()}
    refs = shm.SharedBlocks(os.getpid(), os.getpid(), call.id, "out").share(outputs)
    result: dict[str, object] = {"method": call.method}
    tags = call.args.get("tags")
    if isinstance(tags, list):
        result["tags"] = [f"{tag}!" for tag in tags]
    return Reply(call.id, arrays=refs, result=result, duration_ms=1.0)


def failing(kind: ErrorKind, when: Callable[[Call, int], bool]) -> Behaviour:
    def behaviour(call: Call, arrays: dict[str, np.ndarray]) -> Reply:
        rows = next(iter(arrays.values())).shape[0] if arrays else 1
        if when(call, rows):
            return Reply(call.id, error_kind=kind, error=f"{kind} for {rows} rows")
        return echo(call, arrays)

    return behaviour


@dataclass(eq=False)
class FakeEngine:
    name: str
    behaviour: Behaviour = echo
    latency_s: float = 0.0
    pid: int = field(default_factory=os.getpid)
    alive: bool = True
    calls: list[Call] = field(default_factory=list)
    rows: list[int] = field(default_factory=list)

    async def call(self, call: Call) -> Reply:
        self.calls.append(call)
        arrays = shm.read_all(call.arrays)
        self.rows.append(next(iter(arrays.values())).shape[0] if arrays else -1)
        hang = call.args.get("hang")
        if isinstance(hang, list) and any(hang):
            await asyncio.sleep(max(0.0, call.deadline_at - time.monotonic()))
            raise EngineKilled(self.name, CAUSE_DEADLINE)
        if self.latency_s:
            await asyncio.sleep(self.latency_s)
        return self.behaviour(call, arrays)


class FakePool:
    def __init__(self, *engines: FakeEngine, state: str = "ready") -> None:
        self.engines = list(engines)
        self.state = state
        self.busy: set[str] = set()
        self.oom_reports: list[str] = []
        self._changed = asyncio.Event()

    async def acquire(self, slot: str, deadline_at: float) -> FakeEngine:
        while True:
            if self.state != "ready":
                raise SlotUnavailable(slot, self.state)
            for engine in self.engines:
                if engine.name not in self.busy:
                    self.busy.add(engine.name)
                    return engine
            remaining = deadline_at - time.monotonic()
            if remaining <= 0:
                raise DeadlineExceeded("acquire")
            waiter = asyncio.create_task(self._changed.wait())
            await asyncio.wait({waiter}, timeout=remaining)
            waiter.cancel()

    def release(self, engine: FakeEngine) -> None:
        self.busy.discard(engine.name)
        self._changed.set()
        self._changed = asyncio.Event()

    def report_oom(self, slot: str, client: FakeEngine) -> None:
        self.oom_reports.append(client.name)


def batcher(pool: FakePool, max_batch: int = 8, max_wait_ms: int = 20) -> Batcher:
    return Batcher("fake", max_batch, max_wait_ms, pool, Counters(), log=quiet_log())


def rows(n: int, start: float = 0.0) -> np.ndarray:
    return np.arange(n * 2, dtype=np.float32).reshape(n, 2) + start


def soon(seconds: float = 5.0) -> float:
    return time.monotonic() + seconds


async def test_rows_of_different_submissions_share_one_call() -> None:
    engine = FakeEngine("e")
    pool = FakePool(engine)
    b = batcher(pool, max_batch=8, max_wait_ms=30)
    try:
        first, second = await asyncio.gather(
            b.submit("echo", {"x": rows(3)}, {"tags": ["a", "b", "c"]}, soon()),
            b.submit("echo", {"x": rows(2, 100)}, {"tags": ["d", "e"]}, soon()),
        )
        assert engine.rows == [5]
        np.testing.assert_array_equal(first[0]["x"], rows(3) * 2)
        np.testing.assert_array_equal(second[0]["x"], rows(2, 100) * 2)
        assert first[1] == {"method": "echo", "tags": ["a!", "b!", "c!"]}
        assert second[1] == {"method": "echo", "tags": ["d!", "e!"]}
        assert not any(
            entry.name.startswith("wk-") and f"-{os.getpid()}-" in entry.name
            for entry in shm.SHM_DIR.iterdir()
        )
    finally:
        await b.close()


async def test_large_submission_is_split_by_max_batch_and_reassembled_in_order() -> None:
    engine = FakeEngine("e")
    b = batcher(FakePool(engine), max_batch=4, max_wait_ms=0)
    try:
        arrays, result = await b.submit("echo", {"x": rows(6)}, {"tags": list("abcdef")}, soon())
        assert engine.rows == [4, 2]
        np.testing.assert_array_equal(arrays["x"], rows(6) * 2)
        assert result["tags"] == ["a!", "b!", "c!", "d!", "e!", "f!"]
    finally:
        await b.close()


async def test_different_forms_never_share_a_call() -> None:
    engine = FakeEngine("e")
    b = batcher(FakePool(engine), max_batch=8, max_wait_ms=30)
    try:
        await asyncio.gather(
            b.submit("echo", {"x": rows(1)}, {"kind": "query"}, soon()),
            b.submit("echo", {"x": rows(1)}, {"kind": "document"}, soon()),
            b.submit("echo", {"x": np.ones((1, 3), np.float32)}, {"kind": "query"}, soon()),
            b.submit("other", {"x": rows(1)}, {"kind": "query"}, soon()),
        )
        assert sorted(engine.rows) == [1, 1, 1, 1]
        assert {(c.method, c.args["kind"]) for c in engine.calls} == {
            ("echo", "query"),
            ("echo", "document"),
            ("other", "query"),
        }
    finally:
        await b.close()


async def test_cost_budget_packs_by_tokens_longest_first() -> None:
    engine = FakeEngine("e")
    b = batcher(FakePool(engine), max_batch=10, max_wait_ms=30)
    try:
        await asyncio.gather(
            b.submit("embed", {}, {"texts": ["aaaaaa"]}, soon(), costs=[6]),
            b.submit("embed", {}, {"texts": ["bbbbbb"]}, soon(), costs=[6]),
            b.submit("embed", {}, {"texts": ["ccc"]}, soon(), costs=[3]),
        )
        assert [c.args["texts"] for c in engine.calls] == [["aaaaaa", "ccc"], ["bbbbbb"]]
    finally:
        await b.close()


async def test_priority_rows_jump_the_audio_queue() -> None:
    engine = FakeEngine("e", latency_s=0.03)
    b = batcher(FakePool(engine), max_batch=8, max_wait_ms=5)
    try:
        flood = [
            asyncio.create_task(b.submit("audio", {"x": rows(1)}, {}, soon(20))) for _ in range(40)
        ]
        await asyncio.sleep(0.02)
        latencies = []
        for _ in range(5):
            started = time.monotonic()
            await b.submit("encode", {"x": rows(1)}, {}, soon(20), priority=True)
            latencies.append(time.monotonic() - started)
        await asyncio.gather(*flood)
        assert max(latencies) < 1.0
        methods = [c.method for c in engine.calls]
        first_encode = methods.index("encode")
        assert first_encode <= 2
        assert sum(1 for m in methods[first_encode:] if m == "audio") >= 1
    finally:
        await b.close()


async def test_oom_halves_batch_until_it_fits_and_shrinks_limit() -> None:
    engine = FakeEngine("e", failing(ErrorKind.OOM, lambda _, n: n > 2))
    pool = FakePool(engine)
    b = batcher(pool, max_batch=8, max_wait_ms=0)
    try:
        arrays, _ = await b.submit("echo", {"x": rows(8)}, {}, soon())
        np.testing.assert_array_equal(arrays["x"], rows(8) * 2)
        assert engine.rows[0] == 8
        assert sorted(engine.rows) == [2, 2, 2, 2, 4, 4, 8]
        assert pool.oom_reports == ["e", "e", "e"]
        assert b.limit == 3
    finally:
        await b.close()


async def test_single_row_oom_fails_only_that_row() -> None:
    engine = FakeEngine("e", failing(ErrorKind.OOM, lambda _, n: True))
    b = batcher(FakePool(engine), max_batch=1, max_wait_ms=0)
    try:
        with pytest.raises(EngineError) as raised:
            await b.submit("echo", {"x": rows(1)}, {}, soon())
        assert raised.value.kind is ErrorKind.OOM
    finally:
        await b.close()


async def test_bad_input_is_isolated_to_the_guilty_submission() -> None:
    engine = FakeEngine(
        "e", failing(ErrorKind.BAD_INPUT, lambda call, _: "bad" in list(call.args["tags"]))
    )
    b = batcher(FakePool(engine), max_batch=8, max_wait_ms=30)
    try:
        good, bad = await asyncio.gather(
            b.submit("echo", {"x": rows(2)}, {"tags": ["ok", "ok"]}, soon()),
            b.submit("echo", {"x": rows(1)}, {"tags": ["bad"]}, soon()),
            return_exceptions=True,
        )
        assert isinstance(good, tuple) and good[1]["tags"] == ["ok!", "ok!"]
        assert isinstance(bad, EngineError) and bad.kind is ErrorKind.BAD_INPUT
        assert engine.rows[0] == 3 and sorted(engine.rows[1:]) == [1, 2]
    finally:
        await b.close()


async def test_innocent_neighbour_of_a_poisoner_is_rerun_solo_and_succeeds() -> None:
    engine = FakeEngine("e")
    b = batcher(FakePool(engine), max_batch=8, max_wait_ms=30)
    try:
        poison = asyncio.create_task(
            b.submit("echo", {"x": rows(1)}, {"hang": [True], "tags": ["p"]}, soon(0.3))
        )
        innocent = asyncio.create_task(
            b.submit("echo", {"x": rows(1, 10)}, {"hang": [False], "tags": ["i"]}, soon(5))
        )
        with pytest.raises(DeadlineExceeded):
            await poison
        arrays, result = await innocent
        np.testing.assert_array_equal(arrays["x"], rows(1, 10) * 2)
        assert result["tags"] == ["i!"]
        assert engine.rows == [2, 1]
        assert engine.calls[1].args["tags"] == ["i"]
    finally:
        await b.close()


async def test_engine_crash_fails_every_row_of_the_call() -> None:
    def crash(call: Call, arrays: dict[str, np.ndarray]) -> Reply:
        raise EngineCrashed("e", "exit code 7")

    b = batcher(FakePool(FakeEngine("e", crash)), max_batch=8, max_wait_ms=20)
    try:
        results = await asyncio.gather(
            b.submit("echo", {"x": rows(1)}, {}, soon()),
            b.submit("echo", {"x": rows(1)}, {}, soon()),
            return_exceptions=True,
        )
        assert all(isinstance(result, EngineCrashed) for result in results)
    finally:
        await b.close()


async def test_broken_slot_fails_fast() -> None:
    b = batcher(FakePool(FakeEngine("e"), state="broken"), max_wait_ms=0)
    try:
        with pytest.raises(SlotUnavailable) as raised:
            await b.submit("echo", {"x": rows(1)}, {}, soon())
        assert raised.value.state == "broken"
    finally:
        await b.close()


async def test_queued_row_expires_without_a_call() -> None:
    engine = FakeEngine("e")
    pool = FakePool(engine)
    pool.busy.add("e")
    b = batcher(pool, max_wait_ms=0)
    try:
        with pytest.raises(DeadlineExceeded):
            await b.submit("echo", {"x": rows(1)}, {}, soon(0.1))
        assert engine.calls == []
    finally:
        await b.close()


async def test_reply_with_wrong_row_count_is_a_model_error() -> None:
    def wrong(call: Call, arrays: dict[str, np.ndarray]) -> Reply:
        refs = shm.SharedBlocks(os.getpid(), os.getpid(), call.id, "out").share(
            {"x": np.zeros((5, 2), np.float32)}
        )
        return Reply(call.id, arrays=refs)

    b = batcher(FakePool(FakeEngine("e", wrong)), max_wait_ms=0)
    try:
        with pytest.raises(EngineError) as raised:
            await b.submit("echo", {"x": rows(2)}, {}, soon())
        assert raised.value.kind is ErrorKind.MODEL_ERROR
    finally:
        await b.close()


async def test_close_fails_queued_rows() -> None:
    pool = FakePool(FakeEngine("e"))
    pool.busy.add("e")
    b = batcher(pool, max_wait_ms=0)
    pending = asyncio.create_task(b.submit("echo", {"x": rows(1)}, {}, soon()))
    await asyncio.sleep(0.02)
    await b.close()
    with pytest.raises(SlotUnavailable):
        await pending
    with pytest.raises(SlotUnavailable):
        await b.submit("echo", {"x": rows(1)}, {}, soon())


async def test_shared_results_and_per_row_lists_are_reassembled() -> None:
    def scored(call: Call, arrays: dict[str, np.ndarray]) -> Reply:
        n = next(iter(arrays.values())).shape[0]
        return Reply(call.id, result={"scores": [float(i) for i in range(n)], "model": "m"})

    engine = FakeEngine("e", scored)
    b = batcher(FakePool(engine), max_batch=2, max_wait_ms=0)
    try:
        arrays, result = await b.submit("score", {"x": rows(3)}, {"language": "ru"}, soon())
        assert arrays == {}
        assert result == {"model": "m", "scores": [0.0, 1.0, 0.0]}
        assert all(c.args == {"language": "ru"} for c in engine.calls)
    finally:
        await b.close()


def test_prepare_rejects_mismatched_rows() -> None:
    with pytest.raises(ValueError, match="leading batch axis"):
        prepare("m", {"a": np.ones((2, 1)), "b": np.ones((3, 1))}, {}, 0.0)
    with pytest.raises(ValueError, match="per-row lists"):
        prepare("m", {"a": np.ones((2, 1))}, {"tags": ["x"]}, 0.0)
    with pytest.raises(ValueError, match="per-row lists"):
        prepare("m", {}, {"tags": ["x"], "langs": ["a", "b"]}, 0.0)


async def test_prepare_infers_rows_from_lists_and_defaults_to_one() -> None:
    assert prepare("m", {}, {"texts": ["a", "b"]}, 0.0).rows == 2
    single = prepare("m", {}, {"prompt": "hi", "schema": ("k",)}, 0.0)
    assert single.rows == 1 and single.scalars == {"prompt": "hi", "schema": ("k",)}
    with_arrays = prepare("m", {"clip": np.ones((1, 16000))}, {"tokens": [["a", "b"]]}, 0.0)
    assert with_arrays.rows == 1 and with_arrays.per_row == {"tokens": [["a", "b"]]}


async def test_rows_of_a_cancelled_submission_never_reach_the_engine() -> None:
    engine = FakeEngine("e", latency_s=0.3)
    b = batcher(FakePool(engine), max_batch=1, max_wait_ms=0)
    try:
        waiting = asyncio.create_task(b.submit("m", {"x": rows(3)}, {}, soon()))
        await asyncio.sleep(0.1)
        waiting.cancel()
        with pytest.raises(asyncio.CancelledError):
            await waiting
        await asyncio.sleep(0.8)
        assert engine.rows == [1]
    finally:
        await b.close()


async def test_engine_restart_restores_the_batch_limit_shrunk_by_oom() -> None:
    engine = FakeEngine("e", failing(ErrorKind.OOM, lambda call, n: n > 1))
    pool = FakePool(engine)
    b = batcher(pool, max_batch=4, max_wait_ms=0)
    try:
        await b.submit("m", {"x": rows(4)}, {}, soon())
        assert b.limit < 4
        engine.alive = False
        fresh = FakeEngine("f")
        pool.engines = [fresh]
        await b.submit("m", {"x": rows(4)}, {}, soon())
        assert b.limit == 4
        assert fresh.rows == [4]
    finally:
        await b.close()
