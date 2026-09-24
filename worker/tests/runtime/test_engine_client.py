from __future__ import annotations

import asyncio
import contextlib
import importlib.util
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pytest

from tests.runtime.support import (
    LOAD_TIMEOUT_S,
    WORKER_ROOT,
    fake_spec,
    launch,
    process_gone,
    quiet_log,
    spawn_client,
    wait_until,
)
from worker.observability.counters import Counters
from worker.runtime import shm
from worker.runtime.clock import MonotonicClock
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    ENGINE_MODULE,
    DeadlineExceeded,
    EngineClient,
    EngineCrashed,
    EngineKilled,
    next_message_id,
)
from worker.runtime.engine_main import EXIT_LOAD_FAILED, EXIT_ORPHANED
from worker.runtime.protocol import Call, ErrorKind


def make_call(
    client: EngineClient,
    method: str,
    arrays: dict[str, np.ndarray],
    args: dict[str, object],
    deadline_s: float = 10.0,
) -> tuple[Call, shm.SharedBlocks]:
    call_id = next_message_id()
    blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
    call = Call(
        id=call_id,
        slot="fake",
        method=method,
        deadline_at=time.monotonic() + deadline_s,
        arrays=blocks.share(arrays),
        args=args,
    )
    return call, blocks


async def test_echo_call_roundtrip_through_shm() -> None:
    counters = Counters()
    client = await spawn_client(fake_spec(), counters)
    try:
        assert client.loaded("fake")
        x = np.arange(12, dtype=np.float32).reshape(3, 4)
        call, blocks = make_call(client, "echo", {"x": x}, {"tags": ["a", "b", "c"], "scale": 3})
        reply = await client.call(call)
        blocks.release()
        assert reply.ok
        assert reply.duration_ms > 0
        outputs = shm.take_all(reply.arrays)
        np.testing.assert_array_equal(outputs["x"], x * 3)
        assert reply.result["tags"] == ["a!", "b!", "c!"]
        assert reply.result["rows"] == 3
        assert reply.result["pid"] == client.pid
        assert not any(
            entry.name.startswith(shm.engine_prefix(os.getpid(), client.pid))
            for entry in shm.SHM_DIR.iterdir()
        )
        assert client.calls == 1
        assert counters.value("slot_calls_total", slot="fake") == 1
        assert not client.busy
    finally:
        await client.stop()


@pytest.mark.parametrize(
    ("method", "kind", "fragment"),
    [
        ("bad", ErrorKind.BAD_INPUT, "bad row"),
        ("boom", ErrorKind.MODEL_ERROR, "model exploded"),
        ("oom", ErrorKind.OOM, "out of memory"),
    ],
)
async def test_model_errors_come_back_as_reply_kinds(
    method: str, kind: ErrorKind, fragment: str
) -> None:
    client = await spawn_client(fake_spec())
    try:
        call, blocks = make_call(client, method, {"x": np.ones((2, 2), np.float32)}, {"fits": 1})
        reply = await client.call(call)
        blocks.release()
        assert reply.error_kind is kind
        assert reply.error is not None and fragment in reply.error
        assert client.alive
    finally:
        await client.stop()


async def test_hanging_call_is_killed_at_deadline_and_frees_shm() -> None:
    client = await spawn_client(fake_spec())
    marker = shm.engine_prefix(os.getpid(), client.pid) + "leftover"
    (shm.SHM_DIR / marker).write_bytes(b"\0" * 16)
    started = time.monotonic()
    call, blocks = make_call(client, "hang", {"x": np.ones((1, 1), np.float32)}, {}, 0.5)
    with pytest.raises(EngineKilled) as raised:
        await client.call(call)
    blocks.release()
    elapsed = time.monotonic() - started
    assert raised.value.cause == CAUSE_DEADLINE
    assert 0.5 <= elapsed <= 1.0
    assert not client.alive
    assert client.exit_code == -9
    assert client.kill_cause == CAUSE_DEADLINE
    assert not (shm.SHM_DIR / marker).exists()
    assert await process_gone(client.pid)
    with pytest.raises(EngineCrashed):
        await client.call(call)


@pytest.mark.parametrize(("method", "code"), [("crash", 7), ("segv", -11)])
async def test_sudden_death_fails_pending_call_with_crash(method: str, code: int) -> None:
    client = await spawn_client(fake_spec())
    call, blocks = make_call(client, method, {}, {"code": 7})
    with pytest.raises(EngineCrashed) as raised:
        await client.call(call)
    blocks.release()
    assert f"exit code {code}" in str(raised.value)
    assert client.exit_code == code
    assert client.kill_cause is None
    assert await client.exited() == code


async def test_ping_reports_slot_states_and_unload_reload() -> None:
    client = await spawn_client(fake_spec())
    try:
        call, blocks = make_call(client, "echo", {}, {})
        await client.call(call)
        blocks.release()
        pong = await client.ping(5.0)
        [state] = pong.slots
        assert state.slot == "fake" and state.loaded and state.calls == 1
        assert state.reserved_mib == 0 and state.allocated_mib == 0
        unloaded = await client.unload("fake", 5.0)
        assert not unloaded.loaded
        assert not client.loaded("fake")
        reloaded = await client.load("fake", LOAD_TIMEOUT_S)
        assert reloaded.loaded
        call, blocks = make_call(client, "echo", {}, {})
        assert (await client.call(call)).ok
        blocks.release()
    finally:
        await client.stop()


async def test_call_on_unloaded_slot_loads_lazily() -> None:
    client = await spawn_client(fake_spec())
    try:
        await client.unload("fake", 5.0)
        call, blocks = make_call(client, "echo", {}, {"tags": ["z"]})
        reply = await client.call(call)
        blocks.release()
        assert reply.ok and reply.result["tags"] == ["z!"]
        assert (await client.ping(5.0)).slots[0].loaded
    finally:
        await client.stop()


async def test_stop_exits_cleanly() -> None:
    client = await spawn_client(fake_spec())
    pid = client.pid
    await client.stop()
    assert client.exit_code == 0
    assert not client.alive
    assert await process_gone(pid)
    await client.stop()


async def test_load_failure_exits_with_load_code() -> None:
    spec = fake_spec(fail_load=True)
    client = EngineClient("broken", (spec,), launch(), Counters(), MonotonicClock(), quiet_log())
    await client.spawn()
    with pytest.raises(EngineCrashed):
        await client.load("fake", LOAD_TIMEOUT_S)
    assert await client.exited() == EXIT_LOAD_FAILED
    assert client.kill_cause is None


@pytest.mark.skipif(importlib.util.find_spec("torch") is None, reason="torch not installed")
async def test_auto_device_is_resolved_inside_the_engine() -> None:
    spec = fake_spec(device="auto")
    client = EngineClient(
        "auto",
        (spec,),
        launch(CUDA_VISIBLE_DEVICES=""),
        Counters(),
        MonotonicClock(),
        quiet_log(),
    )
    await client.spawn()
    try:
        await client.load("fake", LOAD_TIMEOUT_S)
        call, blocks = make_call(client, "echo", {}, {})
        reply = await client.call(call)
        blocks.release()
        assert reply.ok and reply.result["device"] == "cpu"
    finally:
        await client.stop()


async def test_slow_load_times_out_and_can_be_killed() -> None:
    spec = fake_spec(load_delay_s=5)
    client = EngineClient("slow", (spec,), launch(), Counters(), MonotonicClock(), quiet_log())
    await client.spawn()
    with pytest.raises(TimeoutError):
        await client.load("fake", 0.3)
    await client.kill("load-timeout")
    assert client.exit_code == -9
    assert await process_gone(client.pid)


class Unpicklable:
    def __reduce__(self) -> str:
        raise TypeError("cannot pickle this thing")


async def test_unsendable_call_leaves_the_engine_idle() -> None:
    client = await spawn_client(fake_spec())
    try:
        call, blocks = make_call(client, "echo", {}, {"thing": Unpicklable()})
        with pytest.raises(TypeError):
            await client.call(call)
        blocks.release()
        assert not client.busy
        call, blocks = make_call(client, "echo", {"x": np.ones((1, 2), np.float32)}, {})
        reply = await client.call(call)
        blocks.release()
        shm.take_all(reply.arrays)
        assert reply.ok
    finally:
        await client.stop()


async def test_call_past_its_deadline_is_refused_without_a_kill() -> None:
    client = await spawn_client(fake_spec())
    try:
        call, blocks = make_call(client, "echo", {"x": np.ones((1, 2), np.float32)}, {}, -1.0)
        with pytest.raises(DeadlineExceeded):
            await client.call(call)
        blocks.release()
        await asyncio.sleep(0.2)
        assert client.alive and not client.busy
    finally:
        await client.stop()


async def test_stop_spends_one_grace_budget_for_reply_and_exit() -> None:
    spec = fake_spec(unload_delay_s=1.5, exit_delay_s=10.0)
    client = EngineClient(
        "engine", (spec,), launch(), Counters(), MonotonicClock(), quiet_log(), stop_grace_s=2.0
    )
    await client.spawn()
    await client.load(spec.name, LOAD_TIMEOUT_S)
    started = time.monotonic()
    await client.stop()
    assert time.monotonic() - started < 2.0 + 0.8
    assert await process_gone(client.pid)


async def test_sudden_death_kills_the_whole_process_group(tmp_path: Path) -> None:
    client = await spawn_client(fake_spec())
    pid_file = tmp_path / "orphan.pid"
    call, blocks = make_call(client, "orphan", {}, {"pid_file": str(pid_file)})
    try:
        with pytest.raises(EngineCrashed):
            await client.call(call)
        assert await process_gone(int(pid_file.read_text(encoding="ascii")), 3.0)
    finally:
        blocks.release()
        if pid_file.exists():
            with contextlib.suppress(ProcessLookupError):
                os.kill(int(pid_file.read_text(encoding="ascii")), signal.SIGKILL)


async def test_kill_that_cannot_join_fails_pending_calls_at_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    counters = Counters()
    spec = fake_spec()
    client = EngineClient(
        "engine", (spec,), launch(), counters, MonotonicClock(), quiet_log(), kill_join_s=0.3
    )
    await client.spawn()
    await client.load(spec.name, LOAD_TIMEOUT_S)
    pid = client.pid
    monkeypatch.setattr(os, "killpg", lambda pgid, sig: None)
    call, blocks = make_call(client, "hang", {}, {}, deadline_s=0.3)
    try:
        with pytest.raises(EngineKilled) as raised:
            await asyncio.wait_for(client.call(call), 3.0)
        assert raised.value.cause == CAUSE_DEADLINE
        assert not client.alive and not client.busy
        assert await asyncio.wait_for(client.exited(), 1.0) is None
        assert counters.value("engine_kill_join_timeout_total", engine="engine") == 1
        with pytest.raises(EngineCrashed):
            await client.ping(1.0)
    finally:
        blocks.release()
        monkeypatch.undo()
        os.killpg(pid, signal.SIGKILL)
    assert await process_gone(pid)
    assert await wait_until(lambda: client.exit_code == -9, 3.0)


def test_engine_whose_parent_is_not_its_owner_exits_orphaned() -> None:
    parent, child = socket.socketpair()
    with parent, child:
        result = subprocess.run(
            [sys.executable, "-m", ENGINE_MODULE, "--fd", str(child.fileno()), "--owner", "1"],
            pass_fds=(child.fileno(),),
            env={**os.environ, **launch().env},
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    assert result.returncode == EXIT_ORPHANED
    assert "engine_orphaned_at_start" in result.stderr + result.stdout


@pytest.mark.skipif(shutil.which("unshare") is None, reason="unshare not installed")
def test_engine_serves_a_worker_running_as_pid_one() -> None:
    result = subprocess.run(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--mount-proc",
            sys.executable,
            "-c",
            PID_ONE_SCRIPT,
        ],
        cwd=WORKER_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "served-as-pid 1 loaded True" in result.stdout.splitlines()


PID_ONE_SCRIPT = """
import asyncio
import os

from tests.runtime.support import fake_spec, spawn_client


async def serve_as_pid_one():
    client = await spawn_client(fake_spec())
    pong = await client.ping(5.0)
    print("served-as-pid", os.getpid(), "loaded", pong.slots[0].loaded, flush=True)
    await client.stop()


asyncio.run(serve_as_pid_one())
"""
