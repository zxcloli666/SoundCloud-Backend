from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import TypeVar

import nats.errors
import nats.js.errors
from nats.js import JetStreamContext
from nats.js.object_store import ObjectStore

from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure

log = logging.getLogger("worker.bus.objects")

T = TypeVar("T")


class ObjectStores:
    def __init__(self, js: JetStreamContext, max_object_bytes: int | None = None) -> None:
        self._js = js
        self._max_object_bytes = max_object_bytes

    async def get(self, bucket: str, name: str, into: Path, deadline: Deadline) -> int:
        store = await self._open(bucket, deadline)
        info = await self._call(lambda: store.get_info(name), bucket, name, deadline)
        size = info.size or 0
        if self._max_object_bytes is not None and size > self._max_object_bytes:
            raise PermanentFailure(
                Reason.INVALID_REQUEST,
                f"object {bucket}/{name} is {size} bytes > {self._max_object_bytes}",
            )
        with into.open("wb") as handle:
            await self._call(lambda: store.get(name, writeinto=handle), bucket, name, deadline)
        written = into.stat().st_size
        log.info("object_fetched", extra={"bucket": bucket, "object": name, "bytes": written})
        return written

    async def put(self, bucket: str, name: str, source: Path, deadline: Deadline) -> None:
        store = await self._open(bucket, deadline)
        with source.open("rb") as handle:
            await self._call(lambda: store.put(name, handle), bucket, name, deadline)
        log.info(
            "object_stored",
            extra={"bucket": bucket, "object": name, "bytes": source.stat().st_size},
        )

    async def delete(self, bucket: str, name: str, deadline: Deadline) -> None:
        store = await self._open(bucket, deadline)
        try:
            await self._call(lambda: store.delete(name), bucket, name, deadline)
        except PermanentFailure as error:
            if error.reason is not Reason.OBJECT_NOT_FOUND:
                raise
            log.info("object_already_gone", extra={"bucket": bucket, "object": name})
            return
        log.info("object_deleted", extra={"bucket": bucket, "object": name})

    async def _open(self, bucket: str, deadline: Deadline) -> ObjectStore:
        return await self._call(lambda: self._js.object_store(bucket), bucket, None, deadline)

    async def _call(
        self,
        start: Callable[[], Awaitable[T]],
        bucket: str,
        name: str | None,
        deadline: Deadline,
    ) -> T:
        deadline.check("object_store")
        where = f"{bucket}/{name}" if name else bucket
        try:
            return await asyncio.wait_for(start(), timeout=deadline.remaining())
        except nats.js.errors.BucketNotFoundError as error:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"{where}: {error}") from error
        except nats.js.errors.NotFoundError as error:
            raise PermanentFailure(Reason.OBJECT_NOT_FOUND, where) from error
        except nats.js.errors.DigestMismatchError as error:
            raise TransientFailure(
                Reason.OBJECT_STORE_UNAVAILABLE, f"{where}: digest mismatch"
            ) from error
        except nats.errors.TimeoutError as error:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"{where}: {error}") from error
        except TimeoutError as error:
            raise TransientFailure(Reason.DEADLINE_EXCEEDED, "stage=object_store") from error
        except (nats.js.errors.Error, nats.errors.Error, OSError) as error:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"{where}: {error}") from error
