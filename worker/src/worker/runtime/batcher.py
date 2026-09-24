from __future__ import annotations

import asyncio
import math
import os
from collections import deque
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from typing import Protocol

import numpy as np

from worker.observability.counters import Counters
from worker.observability.logging import JsonLog
from worker.runtime import shm
from worker.runtime.clock import Clock, MonotonicClock
from worker.runtime.engine_client import (
    CAUSE_DEADLINE,
    DeadlineExceeded,
    EngineClient,
    EngineCrashed,
    EngineError,
    EngineKilled,
    SlotUnavailable,
    next_message_id,
)
from worker.runtime.protocol import Arrays, Call, ErrorKind, Reply

SOLO_AFTER_RESTART_S = 60.0
OOM_SHRINK = 0.75
STAGE_QUEUE = "queue"
STAGE_CALL = "engine call"

RowResult = tuple[dict[str, np.ndarray], dict[str, object], Mapping[str, object]]


class EnginePool(Protocol):
    async def acquire(self, slot: str, deadline_at: float) -> EngineClient: ...

    def release(self, client: EngineClient) -> None: ...

    def report_oom(self, slot: str, client: EngineClient) -> None: ...


@dataclass(frozen=True)
class Form:
    method: str
    arrays: tuple[tuple[str, str, tuple[int, ...]], ...]
    scalars: tuple[tuple[str, str], ...]
    per_row: tuple[str, ...]


@dataclass(eq=False)
class Submission:
    method: str
    arrays: dict[str, np.ndarray]
    scalars: dict[str, object]
    per_row: dict[str, list[object]]
    rows: int
    deadline_at: float
    form: Form
    done: asyncio.Future[None]
    remaining: int
    results: list[RowResult | None] = field(default_factory=list)
    error: BaseException | None = None

    @property
    def abandoned(self) -> bool:
        return self.error is not None or self.done.cancelled()

    def assemble(self) -> tuple[dict[str, np.ndarray], dict[str, object]]:
        settled = [result for result in self.results if result is not None]
        first_arrays, first_per_row, shared = settled[0]
        arrays = {
            key: np.concatenate([row[0][key] for row in settled], axis=0) for key in first_arrays
        }
        result: dict[str, object] = dict(shared)
        for key in first_per_row:
            result[key] = [row[1][key] for row in settled]
        return arrays, result


@dataclass(eq=False)
class Row:
    submission: Submission
    index: int
    cost: int
    priority: bool
    solo: bool = False
    isolate: bool = False
    cap: int = 0
    settled: bool = False

    @property
    def form(self) -> Form:
        return self.submission.form

    @property
    def deadline_at(self) -> float:
        return self.submission.deadline_at

    def arrays(self) -> dict[str, np.ndarray]:
        return {
            key: value[self.index : self.index + 1] for key, value in self.submission.arrays.items()
        }

    def per_row(self, key: str) -> object:
        return self.submission.per_row[key][self.index]


class Batcher:
    def __init__(
        self,
        slot: str,
        max_batch: int,
        max_wait_ms: int,
        pool: EnginePool,
        counters: Counters,
        *,
        clock: Clock | None = None,
        log: JsonLog | None = None,
    ) -> None:
        self._slot = slot
        self._max_batch = max(1, max_batch)
        self._max_wait_s = max_wait_ms / 1000.0
        self._pool = pool
        self._counters = counters
        self._clock = clock or MonotonicClock()
        self._log = (log or JsonLog()).bind(component="batcher", slot=slot)
        self._high: deque[Row] = deque()
        self._normal: deque[Row] = deque()
        self._wake = asyncio.Event()
        self._limit = self._max_batch
        self._shrunk_by: set[EngineClient] = set()
        self._solo_until = 0.0
        self._loop: asyncio.Task[None] | None = None
        self._inflight: set[asyncio.Task[None]] = set()
        self._closed = False

    @property
    def limit(self) -> int:
        return self._limit

    async def submit(
        self,
        method: str,
        arrays: Arrays,
        args: Mapping[str, object],
        deadline_at: float,
        *,
        costs: Sequence[int] | None = None,
        priority: bool = False,
    ) -> tuple[dict[str, np.ndarray], dict[str, object]]:
        if self._closed:
            raise SlotUnavailable(self._slot, "stopped")
        submission = prepare(method, arrays, args, deadline_at)
        row_costs = normalize_costs(costs, submission.rows)
        queue = self._high if priority else self._normal
        for index in range(submission.rows):
            queue.append(Row(submission, index, row_costs[index], priority))
        if self._loop is None:
            self._loop = asyncio.create_task(self._run(), name=f"batcher:{self._slot}")
        self._wake.set()
        await submission.done
        return submission.assemble()

    async def close(self) -> None:
        await self.stop_intake()
        await asyncio.gather(*self._inflight, return_exceptions=True)
        self._settle_queued()

    async def stop_intake(self) -> None:
        self._closed = True
        self._wake.set()
        if self._loop is not None:
            self._loop.cancel()
            await asyncio.gather(self._loop, return_exceptions=True)
        self._settle_queued()

    def _settle_queued(self) -> None:
        stopped = SlotUnavailable(self._slot, "stopped")
        for queue in (self._high, self._normal):
            while queue:
                self._settle(queue.popleft(), error=stopped)

    async def _run(self) -> None:
        while not self._closed:
            head = self._peek()
            if head is None:
                self._wake.clear()
                await self._wake.wait()
                continue
            self._restore_limit()
            batch = self._collect(head)
            if self._worth_waiting(head, batch):
                await self._clock.sleep(self._max_wait_s)
                head = self._peek()
                if head is None:
                    continue
                batch = self._collect(head)
            self._remove(batch)
            live = self._expire(batch)
            if not live:
                continue
            try:
                client = await self._acquire(live)
            except asyncio.CancelledError:
                stopped = SlotUnavailable(self._slot, "stopped")
                for row in live:
                    self._settle(row, error=stopped)
                raise
            if client is None:
                continue
            task = asyncio.create_task(self._execute(client, live))
            self._inflight.add(task)
            task.add_done_callback(self._inflight.discard)

    def _peek(self) -> Row | None:
        for queue in (self._high, self._normal):
            while queue and queue[0].submission.abandoned:
                self._settle(queue.popleft())
            if queue:
                return queue[0]
        return None

    def _restore_limit(self) -> None:
        if not self._shrunk_by or any(client.alive for client in self._shrunk_by):
            return
        self._shrunk_by.clear()
        self._limit = self._max_batch
        self._log.info("batch_limit_restored", limit=self._limit)

    def _queue_of(self, row: Row) -> deque[Row]:
        return self._high if row.priority else self._normal

    def _collect(self, head: Row) -> list[Row]:
        solo = head.solo or self._clock.now() < self._solo_until
        row_limit = 1 if solo else (head.cap or self._max_batch)
        budget = head.cost if solo else self._limit
        batch: list[Row] = []
        total = 0
        for row in list(self._queue_of(head)):
            if row.submission.abandoned:
                continue
            if row.form != head.form:
                continue
            if (head.isolate or row.isolate) and row.submission is not head.submission:
                continue
            if batch and (
                len(batch) >= row_limit
                or total + row.cost > budget
                or (row.cap and len(batch) >= row.cap)
            ):
                continue
            batch.append(row)
            total += row.cost
            if row.cap:
                row_limit = min(row_limit, row.cap)
            if len(batch) >= row_limit:
                break
        return sorted(batch, key=lambda row: row.cost, reverse=True)

    def _worth_waiting(self, head: Row, batch: list[Row]) -> bool:
        if self._max_wait_s <= 0 or head.solo or self._clock.now() < self._solo_until:
            return False
        rows_left = (head.cap or self._max_batch) - len(batch)
        cost_left = self._limit - sum(row.cost for row in batch)
        return rows_left > 0 and cost_left > 0

    def _remove(self, batch: list[Row]) -> None:
        for row in batch:
            self._queue_of(row).remove(row)

    def _expire(self, batch: list[Row]) -> list[Row]:
        now = self._clock.now()
        live: list[Row] = []
        for row in batch:
            if row.submission.abandoned:
                self._settle(row)
            elif row.deadline_at <= now:
                self._settle(row, error=DeadlineExceeded(STAGE_QUEUE))
            else:
                live.append(row)
        return live

    async def _acquire(self, batch: list[Row]) -> EngineClient | None:
        earliest = min(row.deadline_at for row in batch)
        try:
            client = await self._pool.acquire(self._slot, earliest)
        except SlotUnavailable as error:
            for row in batch:
                self._settle(row, error=error)
            return None
        except DeadlineExceeded:
            self._requeue(self._expire(batch), front=True)
            return None
        live = self._expire(batch)
        if len(live) != len(batch):
            self._pool.release(client)
            self._requeue(live, front=True)
            return None
        return client

    async def _execute(self, client: EngineClient, batch: list[Row]) -> None:
        call_id = next_message_id()
        blocks = shm.SharedBlocks(os.getpid(), client.pid, call_id, "in")
        self._counters.observe("slot_batch_rows", len(batch), slot=self._slot)
        try:
            call = build_call(call_id, self._slot, batch, blocks)
            reply = await client.call(call)
        except EngineKilled as killed:
            self._after_kill(batch, killed)
            return
        except DeadlineExceeded as late:
            for row in batch:
                self._settle(row, error=late)
            return
        except EngineCrashed as crashed:
            for row in batch:
                self._settle(row, error=crashed)
            return
        except Exception as error:
            self._log.exception("batch_call_failed", error, engine=client.name, rows=len(batch))
            failure = EngineCrashed(client.name, f"call could not be made: {error}")
            for row in batch:
                self._settle(row, error=failure)
            return
        finally:
            blocks.release()
            self._pool.release(client)
        self._after_reply(client, batch, reply)

    def _after_kill(self, batch: list[Row], killed: EngineKilled) -> None:
        now = self._clock.now()
        expired = [row for row in batch if row.deadline_at <= now]
        innocent = [row for row in batch if row.deadline_at > now]
        for row in expired:
            self._settle(row, error=DeadlineExceeded(STAGE_CALL))
        for row in innocent:
            row.solo = True
        self._requeue(innocent, front=True)
        if killed.cause == CAUSE_DEADLINE:
            self._solo_until = now + SOLO_AFTER_RESTART_S
            self._shrunk_by.clear()
            self._limit = self._max_batch
        self._log.warning(
            "batch_killed",
            engine=killed.engine,
            cause=killed.cause,
            expired=len(expired),
            innocent=len(innocent),
        )

    def _after_reply(self, client: EngineClient, batch: list[Row], reply: Reply) -> None:
        if reply.ok:
            self._distribute(client, batch, reply)
            return
        assert reply.error_kind is not None
        message = reply.error or ""
        if reply.error_kind is ErrorKind.OOM:
            self._pool.report_oom(self._slot, client)
            self._shrunk_by.add(client)
            self._limit = max(1, int(self._limit * OOM_SHRINK))
            if len(batch) > 1:
                cap = math.ceil(len(batch) / 2)
                for row in batch:
                    row.cap = cap
                self._requeue(batch, front=True)
                self._log.warning("batch_oom_split", rows=len(batch), cap=cap, limit=self._limit)
                return
        elif len({id(row.submission) for row in batch}) > 1:
            for row in batch:
                row.isolate = True
            self._requeue(batch, front=True)
            self._log.warning("batch_isolated", rows=len(batch), kind=str(reply.error_kind))
            return
        error = EngineError(reply.error_kind, message)
        for row in batch:
            self._settle(row, error=error)

    def _distribute(self, client: EngineClient, batch: list[Row], reply: Reply) -> None:
        try:
            outputs = shm.take_all(reply.arrays)
        except OSError as error:
            crashed = EngineCrashed(client.name, f"reply arrays unreadable: {error}")
            for row in batch:
                self._settle(row, error=crashed)
            return
        rows = len(batch)
        for key, array in outputs.items():
            if array.shape[:1] != (rows,):
                mismatch = EngineError(
                    ErrorKind.MODEL_ERROR,
                    f"reply array {key} has shape {array.shape} for {rows} rows",
                )
                for row in batch:
                    self._settle(row, error=mismatch)
                return
        per_row = {
            key: value
            for key, value in reply.result.items()
            if isinstance(value, list) and len(value) == rows
        }
        shared = {key: value for key, value in reply.result.items() if key not in per_row}
        for position, row in enumerate(batch):
            row_arrays = {key: array[position : position + 1] for key, array in outputs.items()}
            row_result = {key: value[position] for key, value in per_row.items()}
            self._settle(row, result=(row_arrays, row_result, shared))

    def _requeue(self, rows: list[Row], *, front: bool) -> None:
        if self._closed:
            stopped = SlotUnavailable(self._slot, "stopped")
            for row in rows:
                self._settle(row, error=stopped)
            return
        for row in reversed(rows) if front else rows:
            if front:
                self._queue_of(row).appendleft(row)
            else:
                self._queue_of(row).append(row)
        self._wake.set()

    def _settle(
        self, row: Row, *, result: RowResult | None = None, error: BaseException | None = None
    ) -> None:
        if row.settled:
            return
        row.settled = True
        submission = row.submission
        if result is not None:
            submission.results[row.index] = result
        elif error is not None and submission.error is None:
            submission.error = error
        submission.remaining -= 1
        if submission.remaining > 0 or submission.done.done():
            return
        if submission.error is not None:
            submission.done.set_exception(submission.error)
        else:
            submission.done.set_result(None)


def prepare(
    method: str, arrays: Arrays, args: Mapping[str, object], deadline_at: float
) -> Submission:
    materialized = {key: np.asarray(value) for key, value in arrays.items()}
    objects = sorted(key for key, value in materialized.items() if value.dtype.hasobject)
    if objects:
        raise ValueError(f"{method}: object arrays {objects} cannot be shared")
    leading = {value.shape[0] if value.ndim else -1 for value in materialized.values()}
    if len(leading) > 1 or -1 in leading:
        raise ValueError(f"{method}: arrays must share a leading batch axis, got {leading}")
    lists = {key: value for key, value in args.items() if isinstance(value, list)}
    lengths = {len(value) for value in lists.values()}
    if leading:
        rows = leading.pop()
    elif lengths:
        rows = lengths.pop()
    else:
        rows = 1
    if rows < 1:
        raise ValueError(f"{method}: empty submission")
    short = sorted(key for key, value in lists.items() if len(value) != rows)
    if short:
        raise ValueError(f"{method}: per-row lists {short} must have {rows} entries")
    scalars = {key: value for key, value in args.items() if key not in lists}
    form = Form(
        method,
        tuple(
            sorted((key, value.dtype.str, value.shape[1:]) for key, value in materialized.items())
        ),
        tuple(sorted((key, repr(value)) for key, value in scalars.items())),
        tuple(sorted(lists)),
    )
    return Submission(
        method=method,
        arrays=materialized,
        scalars=scalars,
        per_row={key: list(value) for key, value in lists.items()},
        rows=rows,
        deadline_at=deadline_at,
        form=form,
        done=asyncio.get_running_loop().create_future(),
        remaining=rows,
        results=[None] * rows,
    )


def normalize_costs(costs: Sequence[int] | None, rows: int) -> list[int]:
    if costs is None:
        return [1] * rows
    if len(costs) != rows:
        raise ValueError(f"costs has {len(costs)} entries for {rows} rows")
    return [max(1, int(cost)) for cost in costs]


def build_call(call_id: int, slot: str, batch: list[Row], blocks: shm.SharedBlocks) -> Call:
    head = batch[0].submission
    arrays = {
        key: np.concatenate([row.arrays()[key] for row in batch], axis=0) for key in head.arrays
    }
    args: dict[str, object] = dict(head.scalars)
    for key in head.per_row:
        args[key] = [row.per_row(key) for row in batch]
    return Call(
        id=call_id,
        slot=slot,
        method=head.method,
        deadline_at=min(row.deadline_at for row in batch),
        arrays=blocks.share(arrays),
        args=args,
    )
