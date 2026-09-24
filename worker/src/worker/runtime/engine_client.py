from __future__ import annotations

import asyncio
import os
import signal
import socket
import sys
from asyncio.subprocess import Process
from collections.abc import Mapping
from dataclasses import dataclass, field, replace
from itertools import count
from multiprocessing.connection import Connection

from worker.observability.counters import Counters
from worker.observability.logging import JsonLog
from worker.runtime import shm
from worker.runtime.clock import Clock
from worker.runtime.protocol import (
    Call,
    Command,
    CommandKind,
    ErrorKind,
    Pong,
    Reply,
    SlotSpec,
    SlotState,
)

ENGINE_MODULE = "worker.runtime.engine_main"
DEFAULT_ALLOC_CONF = "expandable_segments:True"
CAUSE_DEADLINE = "deadline"
CAUSE_PING = "ping"
CAUSE_RECYCLE = "recycle"
CAUSE_STOP = "stop"
CAUSE_LOAD_TIMEOUT = "load-timeout"
MESSAGE_IDS = count(1)


def next_message_id() -> int:
    return next(MESSAGE_IDS)


class EngineKilled(Exception):
    def __init__(self, engine: str, cause: str) -> None:
        super().__init__(f"engine {engine} killed: {cause}")
        self.engine = engine
        self.cause = cause


class EngineCrashed(Exception):
    def __init__(self, engine: str, detail: str) -> None:
        super().__init__(f"engine {engine} crashed: {detail}")
        self.engine = engine
        self.detail = detail


class EngineError(Exception):
    def __init__(self, kind: ErrorKind, message: str) -> None:
        super().__init__(f"{kind}: {message}")
        self.kind = kind
        self.message = message


class DeadlineExceeded(Exception):
    def __init__(self, stage: str) -> None:
        super().__init__(f"deadline exceeded at {stage}")
        self.stage = stage


class SlotUnavailable(Exception):
    def __init__(self, slot: str, state: str) -> None:
        super().__init__(f"slot {slot} is {state}")
        self.slot = slot
        self.state = state


@dataclass(frozen=True)
class Launch:
    python: str = sys.executable
    onednn: bool = True
    threads: int = 0
    release_after_call: bool = True
    oom_score_adj: int = 900
    env: Mapping[str, str] = field(default_factory=dict)

    def argv(self, fd: int) -> list[str]:
        return [
            self.python,
            "-m",
            ENGINE_MODULE,
            "--fd",
            str(fd),
            "--owner",
            str(os.getpid()),
            "--onednn",
            flag(self.onednn),
            "--threads",
            str(self.threads),
            "--release-after-call",
            flag(self.release_after_call),
            "--oom-score-adj",
            str(self.oom_score_adj),
        ]


def flag(value: bool) -> str:
    return "on" if value else "off"


class EngineClient:
    def __init__(
        self,
        name: str,
        specs: tuple[SlotSpec, ...],
        launch: Launch,
        counters: Counters,
        clock: Clock,
        log: JsonLog,
        *,
        kill_join_s: float = 2.0,
        stop_grace_s: float = 5.0,
    ) -> None:
        self.name = name
        self.specs = tuple(replace(spec, options=dict(spec.options)) for spec in specs)
        self._launch = launch
        self._counters = counters
        self._clock = clock
        self._log = log.bind(engine=name)
        self._kill_join_s = kill_join_s
        self._stop_grace_s = stop_grace_s
        self._process: Process | None = None
        self._conn: Connection | None = None
        self._pending: dict[int, asyncio.Future[object]] = {}
        self._call_slots: dict[int, str] = {}
        self._watchers: dict[int, asyncio.Task[None]] = {}
        self._reaper: asyncio.Task[None] | None = None
        self._exit: asyncio.Future[int] = asyncio.get_running_loop().create_future()
        self._gone: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        self._kill_cause: str | None = None
        self._dead = False
        self._states = {spec.name: SlotState(spec.name, False, 0, 0, 0) for spec in self.specs}
        self._last_call_at = {spec.name: clock.now() for spec in self.specs}
        self._calls = 0

    @property
    def pid(self) -> int:
        return self._process.pid if self._process is not None else 0

    @property
    def alive(self) -> bool:
        return self._process is not None and not self._dead

    @property
    def busy(self) -> bool:
        return bool(self._pending)

    @property
    def kill_cause(self) -> str | None:
        return self._kill_cause

    @property
    def exit_code(self) -> int | None:
        return self._exit.result() if self._exit.done() else None

    @property
    def calls(self) -> int:
        return self._calls

    @property
    def slot_states(self) -> Mapping[str, SlotState]:
        return self._states

    def loaded(self, slot: str) -> bool:
        return self._states[slot].loaded

    def last_call_at(self, slot: str) -> float:
        return self._last_call_at[slot]

    async def spawn(self) -> None:
        parent, child = socket.socketpair()
        env = {**os.environ, **self._launch.env}
        env.setdefault("PYTORCH_CUDA_ALLOC_CONF", DEFAULT_ALLOC_CONF)
        with child:
            self._process = await asyncio.create_subprocess_exec(
                *self._launch.argv(child.fileno()),
                pass_fds=(child.fileno(),),
                start_new_session=True,
                stdin=asyncio.subprocess.DEVNULL,
                env=env,
            )
        self._conn = Connection(parent.detach())
        asyncio.get_running_loop().add_reader(self._conn.fileno(), self._on_readable)
        self._reaper = asyncio.create_task(self._reap(), name=f"reap:{self.name}")
        self._send(self.specs)
        self._log.info("engine_spawned", pid=self.pid, slots=[spec.name for spec in self.specs])

    async def load(self, slot: str, timeout_s: float) -> SlotState:
        await self.command(CommandKind.LOAD, slot, timeout_s)
        return self._states[slot]

    async def unload(self, slot: str, timeout_s: float) -> SlotState:
        await self.command(CommandKind.UNLOAD, slot, timeout_s)
        return self._states[slot]

    async def ping(self, timeout_s: float) -> Pong:
        return await self.command(CommandKind.PING, None, timeout_s)

    async def command(self, kind: CommandKind, slot: str | None, timeout_s: float) -> Pong:
        self._ensure_running()
        command = Command(next_message_id(), kind, slot)
        future = self._register(command.id)
        try:
            self._send(command)
            message = await asyncio.wait_for(future, timeout_s)
        finally:
            self._pending.pop(command.id, None)
        if not isinstance(message, Pong):
            raise EngineCrashed(self.name, f"{kind} answered with {type(message).__name__}")
        return message

    async def call(self, call: Call) -> Reply:
        self._ensure_running()
        if call.deadline_at <= self._clock.now():
            raise DeadlineExceeded(f"{call.slot} call")
        future = self._register(call.id)
        self._call_slots[call.id] = call.slot
        self._last_call_at[call.slot] = self._clock.now()
        try:
            self._send(call)
        except BaseException:
            self._pending.pop(call.id, None)
            self._call_slots.pop(call.id, None)
            raise
        self._watchers[call.id] = asyncio.create_task(self._watch(call), name=f"watch:{call.id}")
        message = await future
        if not isinstance(message, Reply):
            raise EngineCrashed(self.name, f"call answered with {type(message).__name__}")
        return message

    async def kill(self, cause: str) -> None:
        if self._process is None or self._dead:
            return
        if self._kill_cause is None:
            self._kill_cause = cause
        self._log.warning("engine_kill", pid=self.pid, cause=cause, pending=len(self._pending))
        try:
            os.killpg(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            self._log.info("engine_kill_already_gone", pid=self.pid)
        try:
            await asyncio.wait_for(asyncio.shield(self._exit), self._kill_join_s)
        except TimeoutError:
            self._counters.inc("engine_kill_join_timeout_total", engine=self.name)
            self._log.error("engine_kill_join_timeout", pid=self.pid, cause=cause)
            self._give_up(cause)

    async def stop(self) -> None:
        if not self.alive:
            return
        if self._kill_cause is None:
            self._kill_cause = CAUSE_STOP
        deadline = self._clock.now() + self._stop_grace_s
        try:
            await self.command(CommandKind.STOP, None, self._stop_grace_s)
            remaining = max(0.0, deadline - self._clock.now())
            await asyncio.wait_for(asyncio.shield(self._exit), remaining)
        except (TimeoutError, EngineCrashed, EngineKilled) as error:
            self._log.warning("engine_stop_forced", pid=self.pid, error=str(error))
            await self.kill(CAUSE_STOP)

    async def exited(self) -> int | None:
        await asyncio.shield(self._gone)
        return self.exit_code

    def _register(self, message_id: int) -> asyncio.Future[object]:
        future: asyncio.Future[object] = asyncio.get_running_loop().create_future()
        self._pending[message_id] = future
        return future

    def _ensure_running(self) -> None:
        if self._conn is None or self._dead:
            raise EngineCrashed(self.name, "not running")

    def _send(self, message: object) -> None:
        self._ensure_running()
        assert self._conn is not None
        try:
            self._conn.send(message)
        except OSError as error:
            raise EngineCrashed(self.name, f"send failed: {error}") from error

    def _on_readable(self) -> None:
        if self._conn is None:
            return
        try:
            message = self._conn.recv()
        except (EOFError, OSError):
            asyncio.get_running_loop().remove_reader(self._conn.fileno())
            return
        self._deliver(message)

    def _deliver(self, message: object) -> None:
        message_id = getattr(message, "id", None)
        if not isinstance(message_id, int):
            self._log.error("engine_bad_message", kind=type(message).__name__)
            return
        if isinstance(message, Pong):
            for state in message.slots:
                self._states[state.slot] = state
        elif isinstance(message, Reply):
            self._record_reply(message)
        future = self._pending.pop(message_id, None)
        watcher = self._watchers.pop(message_id, None)
        if watcher is not None:
            watcher.cancel()
        if future is not None and not future.done():
            future.set_result(message)

    def _record_reply(self, reply: Reply) -> None:
        slot = self._call_slots.pop(reply.id, "")
        self._calls += 1
        self._counters.inc("slot_calls_total", slot=slot)
        self._counters.observe("slot_call_ms", reply.duration_ms, slot=slot)

    async def _watch(self, call: Call) -> None:
        await self._clock.sleep(max(0.0, call.deadline_at - self._clock.now()))
        if call.id in self._pending:
            self._log.warning(
                "engine_call_deadline", call=call.id, slot=call.slot, method=call.method
            )
            await self.kill(CAUSE_DEADLINE)

    async def _reap(self) -> None:
        if self._process is None:
            return
        code = await self._process.wait()
        self._kill_orphans()
        self._settle_death(code)

    def _kill_orphans(self) -> None:
        try:
            os.killpg(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            return
        self._counters.inc("engine_orphans_killed_total", engine=self.name)
        self._log.warning("engine_orphans_killed", pid=self.pid)

    def _settle_death(self, code: int) -> None:
        if self._exit.done():
            return
        error: Exception = (
            EngineKilled(self.name, self._kill_cause)
            if self._kill_cause is not None
            else EngineCrashed(self.name, f"exit code {code}")
        )
        self._abandon(error)
        swept = shm.sweep(shm.engine_prefix(os.getpid(), self.pid))
        self._log.info(
            "engine_exited", pid=self.pid, code=code, cause=self._kill_cause, shm_swept=swept
        )
        self._exit.set_result(code)

    def _give_up(self, requested: str) -> None:
        cause = self._kill_cause or requested
        self._abandon(EngineKilled(self.name, cause))
        self._log.error("engine_given_up", pid=self.pid, cause=cause)

    def _abandon(self, error: Exception) -> None:
        self._dead = True
        if self._conn is not None:
            asyncio.get_running_loop().remove_reader(self._conn.fileno())
            self._conn.close()
            self._conn = None
        for future in self._pending.values():
            if not future.done():
                future.set_exception(error)
        self._pending.clear()
        self._call_slots.clear()
        for watcher in self._watchers.values():
            watcher.cancel()
        self._watchers.clear()
        self._states = {name: replace(state, loaded=False) for name, state in self._states.items()}
        if not self._gone.done():
            self._gone.set_result(None)
