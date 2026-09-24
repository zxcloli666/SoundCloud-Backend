from __future__ import annotations

import asyncio
import os
from collections import deque
from collections.abc import Coroutine, Iterable, Mapping
from dataclasses import dataclass, field, replace
from typing import Any, cast

from worker.observability.counters import Counters
from worker.observability.logging import JsonLog
from worker.runtime import shm
from worker.runtime.clock import Clock, MonotonicClock
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    CAUSE_LOAD_TIMEOUT,
    CAUSE_PING,
    CAUSE_RECYCLE,
    CAUSE_STOP,
    DeadlineExceeded,
    EngineClient,
    EngineCrashed,
    EngineKilled,
    Launch,
    SlotUnavailable,
)
from worker.runtime.protocol import Pong, SlotSpec

MODE_SLOT = "slot"
MODE_LANE = "lane"
LANE_GROUPS: Mapping[str, tuple[str, ...]] = {
    "audio": ("muq", "mulan", "text"),
    "sync": ("sep", "asr", "align", "mms"),
}

STATE_LOADING = "loading"
STATE_READY = "ready"
STATE_RESTARTING = "restarting"
STATE_BROKEN = "broken"
STATE_UNLOADED = "unloaded"
STATE_STOPPED = "stopped"
PLANNED_CAUSES = frozenset({CAUSE_DEADLINE, CAUSE_PING, CAUSE_RECYCLE, CAUSE_STOP})
GPU_DEVICES = frozenset({"auto", "cuda"})


@dataclass(frozen=True)
class EnginePlan:
    name: str
    slots: tuple[SlotSpec, ...]

    @property
    def slot_names(self) -> tuple[str, ...]:
        return tuple(spec.name for spec in self.slots)


@dataclass(frozen=True)
class RuntimePolicy:
    onednn: bool = True
    threads: int = 0
    release_after_call: bool = True
    recycle_after_calls: int = 10000
    recycle_gap_mib: int = 1024
    recycle_overlap: bool = True
    idle_unload_s: float = 0.0
    oom_unload: bool = True
    oom_recycle_count: int = 3
    oom_recycle_window_s: float = 300.0
    ping_interval_s: float = 5.0
    ping_timeout_s: float = 10.0
    load_timeout_s: float = 900.0
    command_timeout_s: float = 60.0
    respawn_backoff_s: tuple[float, ...] = (1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 60.0)
    breaker_deaths: int = 5
    breaker_window_s: float = 600.0
    breaker_open_s: float = 300.0
    kill_join_s: float = 2.0
    stop_grace_s: float = 5.0


def plan_engines(
    mode: str,
    specs: Mapping[str, SlotSpec],
    replicas: Mapping[str, int],
    groups: Mapping[str, tuple[str, ...]] = LANE_GROUPS,
) -> tuple[EnginePlan, ...]:
    if mode not in (MODE_SLOT, MODE_LANE):
        raise ValueError(f"unknown runtime mode {mode!r}")
    plans: list[EnginePlan] = []
    grouped: set[str] = set()
    if mode == MODE_LANE:
        for group, members in groups.items():
            present = [specs[name] for name in members if name in specs]
            if not present:
                continue
            grouped.update(spec.name for spec in present)
            total = max(replicas.get(spec.name, 1) for spec in present)
            for index in range(total):
                hosted = tuple(spec for spec in present if replicas.get(spec.name, 1) > index)
                plans.append(EnginePlan(replica_name(group, index, total), hosted))
    for name, spec in specs.items():
        if name in grouped:
            continue
        total = replicas.get(name, 1)
        plans.extend(
            EnginePlan(replica_name(name, index, total), (spec,)) for index in range(total)
        )
    return tuple(plans)


def replica_name(base: str, index: int, total: int) -> str:
    return base if total == 1 else f"{base}#{index}"


def on_gpu(device: str) -> bool:
    return device in GPU_DEVICES


@dataclass
class Managed:
    plan: EnginePlan
    client: EngineClient | None = None
    state: str = STATE_LOADING
    deaths: deque[float] = field(default_factory=deque)
    backoff_level: int = 0
    oom_times: deque[float] = field(default_factory=deque)
    recycle_wanted: bool = False
    recycling: bool = False
    task: asyncio.Task[None] | None = None


class Supervisor:
    def __init__(
        self,
        plans: Iterable[EnginePlan],
        policy: RuntimePolicy,
        counters: Counters,
        *,
        launch: Launch | None = None,
        clock: Clock | None = None,
        log: JsonLog | None = None,
    ) -> None:
        self._engines = [Managed(plan) for plan in plans]
        self._policy = policy
        self._counters = counters
        self._launch = launch or Launch()
        self._clock = clock or MonotonicClock()
        self._log = (log or JsonLog()).bind(component="supervisor")
        self._by_slot: dict[str, list[Managed]] = {}
        for managed in self._engines:
            for slot in managed.plan.slot_names:
                self._by_slot.setdefault(slot, []).append(managed)
        self._leased: set[EngineClient] = set()
        self._retiring: set[EngineClient] = set()
        self._changed = asyncio.Event()
        self._housekeeper: asyncio.Task[None] | None = None
        self._side_tasks: set[asyncio.Task[None]] = set()
        self._stopping = False

    @property
    def slots(self) -> tuple[str, ...]:
        return tuple(self._by_slot)

    async def start(self) -> None:
        swept = shm.sweep_dead_owners()
        threads = self._policy.threads or max(
            1, (os.cpu_count() or 1) // max(1, len(self._engines))
        )
        self._launch = replace(
            self._launch,
            threads=threads,
            onednn=self._policy.onednn,
            release_after_call=self._policy.release_after_call,
        )
        self._log.info(
            "supervisor_start", engines=len(self._engines), threads=threads, shm_swept=swept
        )
        for managed in self._engines:
            managed.task = asyncio.create_task(
                self._run(managed), name=f"engine:{managed.plan.name}"
            )
        self._housekeeper = asyncio.create_task(self._housekeeping(), name="housekeeping")

    async def stop(self) -> None:
        self._stopping = True
        self._log.info("supervisor_stop")
        alive = self._alive_clients()
        if self._housekeeper is not None:
            self._housekeeper.cancel()
        for side in list(self._side_tasks):
            side.cancel()
        await asyncio.gather(*(client.stop() for client in alive))
        for managed in self._engines:
            managed.state = STATE_STOPPED
            if managed.task is not None:
                managed.task.cancel()
        tasks = [m.task for m in self._engines if m.task is not None]
        if self._housekeeper is not None:
            tasks.append(self._housekeeper)
        await asyncio.gather(*tasks, *self._side_tasks, return_exceptions=True)
        for client in self._alive_clients():
            await client.kill(CAUSE_STOP)
        swept = shm.sweep(f"{shm.PREFIX}{os.getpid()}-")
        self._notify()
        self._log.info("supervisor_stopped", shm_swept=swept)

    def slot_state(self, slot: str) -> str:
        hosts = self._by_slot.get(slot, [])
        if not hosts:
            return "unknown"
        ready = [m.client for m in hosts if m.state == STATE_READY and m.client is not None]
        if any(client.alive and client.loaded(slot) for client in ready):
            return STATE_READY
        if any(client.alive for client in ready):
            return STATE_UNLOADED
        for state in (STATE_BROKEN, STATE_STOPPED):
            if all(m.state == state for m in hosts):
                return state
        if any(m.state == STATE_LOADING for m in hosts):
            return STATE_LOADING
        return STATE_RESTARTING

    def snapshot(self) -> dict[str, dict[str, object]]:
        latency = cast(
            Mapping[str, Mapping[str, float]],
            self._counters.snapshot()["latency_ms"].get("slot_call_ms", {}),
        )
        report: dict[str, dict[str, object]] = {}
        for slot, hosts in self._by_slot.items():
            quantiles = latency.get(f"slot={slot}", {"p50": 0.0, "p95": 0.0})
            gaps = [
                m.client.slot_states[slot].reserved_mib - m.client.slot_states[slot].allocated_mib
                for m in hosts
                if m.client is not None
            ]
            report[slot] = {
                "state": self.slot_state(slot),
                "restarts": self._counters.value("slot_restarts_total", slot=slot),
                "kills_deadline": self._counters.value("slot_kills_deadline_total", slot=slot),
                "crashes": self._counters.value("slot_crashes_total", slot=slot),
                "oom": self._counters.value("slot_oom_total", slot=slot),
                "reserved_gap_mib": max(gaps, default=0),
                "calls": self._counters.value("slot_calls_total", slot=slot),
                "p50_ms": quantiles["p50"],
                "p95_ms": quantiles["p95"],
            }
        return report

    def engines(self) -> list[tuple[str, int, str]]:
        return [
            (m.plan.name, m.client.pid if m.client is not None else 0, m.state)
            for m in self._engines
        ]

    async def acquire(self, slot: str, deadline_at: float) -> EngineClient:
        hosts = self._by_slot.get(slot)
        if not hosts:
            raise SlotUnavailable(slot, "unknown")
        while True:
            idle = self._idle_clients(hosts)
            if idle:
                client = next((c for c in idle if c.loaded(slot)), idle[0])
                self._leased.add(client)
                if await self._lease(client, slot, deadline_at):
                    return client
                continue
            for state in (STATE_BROKEN, STATE_STOPPED):
                if all(m.state == state for m in hosts):
                    raise SlotUnavailable(slot, state)
            remaining = deadline_at - self._clock.now()
            if remaining <= 0:
                raise DeadlineExceeded(f"acquire {slot}")
            await self._wait_change(remaining)

    def release(self, client: EngineClient) -> None:
        self._leased.discard(client)
        self._notify()

    async def kill(self, slot: str, cause: str) -> int:
        killed = 0
        for managed in self._by_slot.get(slot, []):
            if managed.client is not None and managed.client.alive:
                await managed.client.kill(cause)
                killed += 1
        return killed

    def report_oom(self, slot: str, client: EngineClient) -> None:
        self._counters.inc("slot_oom_total", slot=slot)
        managed = next((m for m in self._by_slot.get(slot, []) if m.client is client), None)
        if managed is None:
            self._log.warning("slot_oom", slot=slot, engine=client.name, current=False)
        else:
            self._count_oom(managed, slot)
        if self._policy.oom_unload:
            device = next(spec.device for spec in client.specs if spec.name == slot)
            self._side_task(self._unload_idlest(slot, device), "oom-unload", slot)

    def _count_oom(self, managed: Managed, slot: str) -> None:
        now = self._clock.now()
        times = managed.oom_times
        times.append(now)
        while times and times[0] < now - self._policy.oom_recycle_window_s:
            times.popleft()
        self._log.warning("slot_oom", slot=slot, engine=managed.plan.name, recent=len(times))
        if len(times) >= self._policy.oom_recycle_count:
            managed.recycle_wanted = True
            times.clear()

    def _alive_clients(self) -> list[EngineClient]:
        managed = [m.client for m in self._engines if m.client is not None]
        return [client for client in (*managed, *self._retiring) if client.alive]

    def _idle_clients(self, hosts: list[Managed]) -> list[EngineClient]:
        return [
            m.client
            for m in hosts
            if m.state == STATE_READY and m.client is not None and self._is_idle(m.client)
        ]

    async def _lease(self, client: EngineClient, slot: str, deadline_at: float) -> bool:
        ready = client.loaded(slot) or await self._load_within(client, slot, deadline_at)
        if ready and deadline_at > self._clock.now():
            return True
        self.release(client)
        if ready:
            raise DeadlineExceeded(f"acquire {slot}")
        return False

    async def _load_within(self, client: EngineClient, slot: str, deadline_at: float) -> bool:
        loading = asyncio.create_task(
            self._load_on_demand(client, slot), name=f"load:{client.name}:{slot}"
        )
        sleeper = asyncio.create_task(self._clock.sleep(max(0.0, deadline_at - self._clock.now())))
        try:
            await asyncio.wait({loading, sleeper}, return_when=asyncio.FIRST_COMPLETED)
        except BaseException:
            self._side_task(self._release_after(loading, client), "release", client.name)
            raise
        finally:
            sleeper.cancel()
        if not loading.done():
            self._side_task(self._release_after(loading, client), "release", client.name)
            raise DeadlineExceeded(f"load {slot}")
        try:
            return loading.result()
        except BaseException:
            self.release(client)
            raise

    async def _release_after(self, loading: asyncio.Task[bool], client: EngineClient) -> None:
        try:
            await loading
        finally:
            self.release(client)

    async def _run(self, managed: Managed) -> None:
        while not self._stopping:
            client = self._new_client(managed)
            self._install(managed, client)
            managed.state = STATE_LOADING
            self._notify()
            try:
                await self._bring_up(client)
            except TimeoutError:
                self._log.error("engine_load_timeout", engine=client.name, pid=client.pid)
                await client.kill(CAUSE_LOAD_TIMEOUT)
            except (EngineCrashed, OSError) as error:
                self._log.error("engine_start_failed", engine=client.name, error=str(error))
            else:
                managed.state = STATE_READY
                self._notify()
            exited = await self._await_current_exit(managed)
            if self._stopping:
                return
            await self._after_death(managed, exited)

    def _new_client(self, managed: Managed) -> EngineClient:
        return EngineClient(
            managed.plan.name,
            managed.plan.slots,
            self._launch,
            self._counters,
            self._clock,
            self._log,
            kill_join_s=self._policy.kill_join_s,
            stop_grace_s=self._policy.stop_grace_s,
        )

    def _install(self, managed: Managed, client: EngineClient) -> None:
        managed.client = client
        managed.oom_times.clear()
        managed.recycle_wanted = False

    async def _bring_up(self, client: EngineClient) -> None:
        await client.spawn()
        for spec in client.specs:
            state = await client.load(spec.name, self._policy.load_timeout_s)
            if not state.loaded:
                raise EngineCrashed(client.name, f"slot {spec.name} reported unloaded after load")
        self._log.info("engine_ready", engine=client.name, pid=client.pid)

    async def _await_current_exit(self, managed: Managed) -> EngineClient:
        while True:
            client = managed.client
            if client is None:
                raise RuntimeError(f"engine {managed.plan.name} has no client")
            if client.pid == 0:
                return client
            await client.exited()
            if managed.client is client:
                return client

    async def _after_death(self, managed: Managed, client: EngineClient) -> None:
        cause = client.kill_cause
        planned = cause in PLANNED_CAUSES
        for slot in managed.plan.slot_names:
            self._counters.inc("slot_restarts_total", slot=slot)
            if cause == CAUSE_DEADLINE:
                self._counters.inc("slot_kills_deadline_total", slot=slot)
            if not planned:
                self._counters.inc("slot_crashes_total", slot=slot)
        managed.state = STATE_RESTARTING
        self._notify()
        if planned:
            managed.backoff_level = 0
            return
        now = self._clock.now()
        managed.deaths.append(now)
        while managed.deaths and managed.deaths[0] < now - self._policy.breaker_window_s:
            managed.deaths.popleft()
        if len(managed.deaths) >= self._policy.breaker_deaths:
            managed.state = STATE_BROKEN
            self._notify()
            self._log.error(
                "engine_broken",
                engine=managed.plan.name,
                deaths=len(managed.deaths),
                open_s=self._policy.breaker_open_s,
            )
            await self._clock.sleep(self._policy.breaker_open_s)
            managed.deaths.clear()
            managed.backoff_level = 0
            return
        backoff = self._policy.respawn_backoff_s
        delay = backoff[min(managed.backoff_level, len(backoff) - 1)]
        managed.backoff_level += 1
        self._log.warning(
            "engine_respawn_backoff",
            engine=managed.plan.name,
            exit_code=client.exit_code,
            cause=cause,
            delay_s=delay,
            deaths=len(managed.deaths),
        )
        await self._clock.sleep(delay)

    async def _housekeeping(self) -> None:
        while not self._stopping:
            await self._clock.sleep(self._policy.ping_interval_s)
            for managed in self._engines:
                client = managed.client
                if (
                    managed.state == STATE_READY
                    and not managed.recycling
                    and client is not None
                    and self._is_idle(client)
                ):
                    await self._inspect_guarded(managed, client)

    def _is_idle(self, client: EngineClient) -> bool:
        return (
            client.alive
            and not client.busy
            and client not in self._leased
            and client not in self._retiring
        )

    async def _inspect_guarded(self, managed: Managed, client: EngineClient) -> None:
        try:
            await self._inspect(managed, client)
        except Exception as error:
            self._counters.inc("supervisor_inspect_errors_total", engine=managed.plan.name)
            self._log.exception("engine_inspect_failed", error, engine=managed.plan.name)
        finally:
            self._notify()

    async def _inspect(self, managed: Managed, client: EngineClient) -> None:
        try:
            pong = await client.ping(self._policy.ping_timeout_s)
        except TimeoutError:
            self._log.error("engine_ping_timeout", engine=client.name, pid=client.pid)
            await client.kill(CAUSE_PING)
            return
        except EngineCrashed as error:
            self._log.warning("engine_ping_failed", engine=client.name, error=str(error))
            return
        gap = self._record_gaps(pong)
        if (
            managed.recycle_wanted
            or client.calls >= self._policy.recycle_after_calls
            or gap > self._policy.recycle_gap_mib
        ):
            managed.recycling = True
            self._side_task(self._recycle(managed, client, gap), "recycle", managed.plan.name)
            return
        if self._policy.idle_unload_s > 0:
            await self._unload_idle(client, pong)

    def _record_gaps(self, pong: Pong) -> int:
        worst = 0
        for state in pong.slots:
            gap = max(0, state.reserved_mib - state.allocated_mib)
            self._counters.gauge("slot_reserved_gap_mib", gap, slot=state.slot)
            worst = max(worst, gap)
        return worst

    async def _recycle(self, managed: Managed, old: EngineClient, gap: int) -> None:
        overlap = self._policy.recycle_overlap and not managed.recycle_wanted
        self._log.info(
            "engine_recycle",
            engine=old.name,
            pid=old.pid,
            calls=old.calls,
            gap_mib=gap,
            oom=managed.recycle_wanted,
            overlap=overlap,
        )
        try:
            if not overlap or not await self._replace(managed, old):
                await self._retire(old)
        finally:
            managed.recycling = False

    async def _replace(self, managed: Managed, old: EngineClient) -> bool:
        fresh = self._new_client(managed)
        try:
            await self._bring_up(fresh)
        except TimeoutError:
            self._recycle_failed(managed, "load timeout")
            await fresh.kill(CAUSE_RECYCLE)
            return False
        except (EngineCrashed, EngineKilled, OSError) as error:
            self._recycle_failed(managed, str(error))
            await self._retire(fresh)
            return False
        except BaseException:
            await fresh.kill(CAUSE_STOP)
            raise
        if self._stopping or managed.client is not old or not old.alive:
            self._log.warning("engine_recycle_superseded", engine=managed.plan.name, pid=fresh.pid)
            await self._retire(fresh)
            return True
        for slot in managed.plan.slot_names:
            self._counters.inc("slot_restarts_total", slot=slot)
        self._install(managed, fresh)
        self._notify()
        await self._retire(old)
        return True

    def _recycle_failed(self, managed: Managed, error: str) -> None:
        self._counters.inc("engine_recycle_failed_total", engine=managed.plan.name)
        self._log.error(
            "engine_recycle_failed", engine=managed.plan.name, error=error, fallback="sequential"
        )

    async def _retire(self, client: EngineClient) -> None:
        self._retiring.add(client)
        try:
            while client.alive and (client.busy or client in self._leased):
                await self._wait_change(self._policy.ping_interval_s)
            await client.stop()
        finally:
            self._retiring.discard(client)

    async def _unload_idle(self, client: EngineClient, pong: Pong) -> None:
        now = self._clock.now()
        for state in pong.slots:
            idle_for = now - client.last_call_at(state.slot)
            if state.loaded and idle_for >= self._policy.idle_unload_s and self._is_idle(client):
                if not await self._unload(client, state.slot):
                    return
                self._log.info(
                    "slot_idle_unloaded",
                    engine=client.name,
                    slot=state.slot,
                    idle_s=round(idle_for, 1),
                )

    async def _load_on_demand(self, client: EngineClient, slot: str) -> bool:
        self._log.info("slot_load_on_demand", engine=client.name, slot=slot)
        try:
            state = await client.load(slot, self._policy.load_timeout_s)
        except TimeoutError:
            self._log.error("slot_load_timeout", engine=client.name, slot=slot)
            await client.kill(CAUSE_LOAD_TIMEOUT)
            return False
        except (EngineCrashed, EngineKilled) as error:
            self._log.error("slot_load_crashed", engine=client.name, slot=slot, error=str(error))
            return False
        self._notify()
        return state.loaded

    async def _unload(self, client: EngineClient, slot: str) -> bool:
        try:
            await client.unload(slot, self._policy.command_timeout_s)
        except TimeoutError:
            self._counters.inc("slot_unload_failed_total", slot=slot)
            self._log.error("slot_unload_timeout", engine=client.name, pid=client.pid, slot=slot)
            await client.kill(CAUSE_PING)
            return False
        except (EngineCrashed, EngineKilled) as error:
            self._counters.inc("slot_unload_failed_total", slot=slot)
            self._log.error("slot_unload_failed", engine=client.name, slot=slot, error=str(error))
            return False
        self._notify()
        return True

    async def _unload_idlest(self, except_slot: str, device: str) -> None:
        now = self._clock.now()
        candidates = [
            (now - m.client.last_call_at(spec.name), m.client, spec.name)
            for m in self._engines
            if m.state == STATE_READY and m.client is not None and self._is_idle(m.client)
            for spec in m.plan.slots
            if spec.name != except_slot
            and on_gpu(spec.device) == on_gpu(device)
            and m.client.loaded(spec.name)
        ]
        if not candidates:
            self._log.info("oom_unload_no_candidate", slot=except_slot, device=device)
            return
        idle_for, client, slot = max(candidates, key=lambda item: item[0])
        if await self._unload(client, slot):
            self._log.warning(
                "oom_unload", engine=client.name, slot=slot, idle_s=round(idle_for, 1)
            )

    def _side_task(self, coroutine: Coroutine[Any, Any, None], kind: str, target: str) -> None:
        task = asyncio.create_task(coroutine, name=f"{kind}:{target}")
        self._side_tasks.add(task)
        task.add_done_callback(self._side_task_done)

    def _side_task_done(self, task: asyncio.Task[None]) -> None:
        self._side_tasks.discard(task)
        error = None if task.cancelled() else task.exception()
        if error is None:
            return
        kind, _, target = task.get_name().partition(":")
        self._counters.inc("supervisor_task_errors_total", task=kind)
        self._log.exception("supervisor_task_failed", error, task=kind, target=target)

    def _notify(self) -> None:
        self._changed.set()
        self._changed = asyncio.Event()

    async def _wait_change(self, timeout_s: float) -> None:
        waiter = asyncio.create_task(self._changed.wait())
        sleeper = asyncio.create_task(self._clock.sleep(timeout_s))
        _, pending = await asyncio.wait({waiter, sleeper}, return_when=asyncio.FIRST_COMPLETED)
        for task in pending:
            task.cancel()
