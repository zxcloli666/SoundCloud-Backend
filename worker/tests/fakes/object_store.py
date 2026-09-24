from __future__ import annotations

import base64
import hashlib
import io
from dataclasses import dataclass, field
from pathlib import Path

import nats.js.errors
from nats.js import api
from nats.js.object_store import ObjectStore

from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure


@dataclass
class FakeObjectStore:
    bucket: str
    objects: dict[str, bytes] = field(default_factory=dict)
    unavailable: bool = False
    corrupt: set[str] = field(default_factory=set)
    gets: list[str] = field(default_factory=list)
    puts: list[str] = field(default_factory=list)

    async def get_info(self, name: str, show_deleted: bool = False) -> api.ObjectInfo:
        self._check_available()
        data = self.objects.get(name)
        if data is None:
            raise nats.js.errors.ObjectNotFoundError(code=404, description="object not found")
        digest = "SHA-256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode()
        return api.ObjectInfo(
            name=name, bucket=self.bucket, nuid=f"nuid-{name}", size=len(data), digest=digest
        )

    async def get(
        self, name: str, writeinto: io.BufferedIOBase | None = None, show_deleted: bool = False
    ) -> ObjectStore.ObjectResult:
        self.gets.append(name)
        info = await self.get_info(name, show_deleted)
        data = self.objects[name]
        if name in self.corrupt:
            raise nats.js.errors.DigestMismatchError
        if writeinto is not None:
            writeinto.write(data)
            return ObjectStore.ObjectResult(info=info, data=b"")
        return ObjectStore.ObjectResult(info=info, data=data)

    async def put(
        self,
        name: str,
        data: str | bytes | io.BufferedIOBase,
        meta: api.ObjectMeta | None = None,
    ) -> api.ObjectInfo:
        self._check_available()
        if isinstance(data, str):
            payload = data.encode()
        elif isinstance(data, bytes):
            payload = data
        else:
            payload = data.read()
        self.objects[name] = payload
        self.puts.append(name)
        return await self.get_info(name)

    async def delete(self, name: str) -> ObjectStore.ObjectResult:
        self._check_available()
        info = await self.get_info(name)
        del self.objects[name]
        return ObjectStore.ObjectResult(info=info, data=b"")

    def _check_available(self) -> None:
        if self.unavailable:
            raise nats.js.errors.ServiceUnavailableError(
                code=503, description="jetstream unavailable"
            )


@dataclass
class FakeBlobStore:
    buckets: dict[str, dict[str, bytes]] = field(default_factory=dict)
    unavailable: bool = False
    refused: set[str] = field(default_factory=set)
    gets: list[tuple[str, str]] = field(default_factory=list)
    puts: list[tuple[str, str]] = field(default_factory=list)
    deletes: list[tuple[str, str]] = field(default_factory=list)

    def add(self, bucket: str, name: str, data: bytes) -> None:
        self.buckets.setdefault(bucket, {})[name] = data

    async def get(self, bucket: str, name: str, into: Path, deadline: Deadline) -> int:
        self.gets.append((bucket, name))
        deadline.check("object_get")
        if self.unavailable:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"bucket={bucket}")
        data = self.buckets.get(bucket, {}).get(name)
        if data is None:
            raise PermanentFailure(Reason.OBJECT_NOT_FOUND, f"{bucket}/{name}")
        into.write_bytes(data)
        return len(data)

    async def put(self, bucket: str, name: str, source: Path, deadline: Deadline) -> None:
        self.puts.append((bucket, name))
        deadline.check("object_put")
        if self.unavailable or name in self.refused:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"bucket={bucket}")
        self.add(bucket, name, source.read_bytes())

    async def delete(self, bucket: str, name: str, deadline: Deadline) -> None:
        self.deletes.append((bucket, name))
        deadline.check("object_delete")
        if self.unavailable:
            raise TransientFailure(Reason.OBJECT_STORE_UNAVAILABLE, f"bucket={bucket}")
        self.buckets.get(bucket, {}).pop(name, None)
