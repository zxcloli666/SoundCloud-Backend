from __future__ import annotations

from pathlib import Path

import nats.js.errors
import pytest

from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeNats
from worker.bus.objects import ObjectStores
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure


@pytest.fixture
def stores(fake_nats: FakeNats) -> ObjectStores:
    return ObjectStores(fake_nats.jetstream(), max_object_bytes=64)


def deadline(clock: FakeClock, seconds: float = 30.0) -> Deadline:
    return Deadline.after(seconds, clock.now)


async def test_get_writes_the_object_to_a_file(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    fake_nats.object_stores["COLLAB_DATA"].objects["collab-input-1"] = b"sessions"
    target = tmp_path / "input.json"
    written = await stores.get("COLLAB_DATA", "collab-input-1", target, deadline(clock))
    assert written == 8 and target.read_bytes() == b"sessions"
    assert fake_nats.object_stores["COLLAB_DATA"].gets == ["collab-input-1"]


async def test_put_uploads_a_file(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    source = tmp_path / "vectors.json"
    source.write_bytes(b'{"dim":128}')
    await stores.put("COLLAB_DATA", "collab-input-1-vectors", source, deadline(clock))
    assert (
        fake_nats.object_stores["COLLAB_DATA"].objects["collab-input-1-vectors"] == b'{"dim":128}'
    )


async def test_missing_object_is_permanent(
    stores: ObjectStores, clock: FakeClock, tmp_path: Path
) -> None:
    with pytest.raises(PermanentFailure) as raised:
        await stores.get("COLLAB_DATA", "nope", tmp_path / "x", deadline(clock))
    assert raised.value.reason is Reason.OBJECT_NOT_FOUND


async def test_missing_bucket_and_unavailable_store_are_transient(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    with pytest.raises(TransientFailure) as raised:
        await stores.get("NOPE", "x", tmp_path / "x", deadline(clock))
    assert raised.value.reason is Reason.OBJECT_STORE_UNAVAILABLE
    fake_nats.object_stores["COLLAB_DATA"].unavailable = True
    (tmp_path / "x").write_bytes(b"x")
    with pytest.raises(TransientFailure) as raised:
        await stores.put("COLLAB_DATA", "x", tmp_path / "x", deadline(clock))
    assert raised.value.reason is Reason.OBJECT_STORE_UNAVAILABLE


async def test_digest_mismatch_is_transient(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    store = fake_nats.object_stores["COLLAB_DATA"]
    store.objects["bad"] = b"data"
    store.corrupt.add("bad")
    with pytest.raises(TransientFailure, match="digest"):
        await stores.get("COLLAB_DATA", "bad", tmp_path / "x", deadline(clock))


async def test_oversized_object_is_invalid_request(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    fake_nats.object_stores["COLLAB_DATA"].objects["big"] = b"x" * 65
    with pytest.raises(PermanentFailure) as raised:
        await stores.get("COLLAB_DATA", "big", tmp_path / "x", deadline(clock))
    assert raised.value.reason is Reason.INVALID_REQUEST
    assert fake_nats.object_stores["COLLAB_DATA"].gets == []


async def test_expired_deadline_is_deadline_exceeded(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    fake_nats.object_stores["COLLAB_DATA"].objects["x"] = b"data"
    with pytest.raises(TransientFailure) as raised:
        await stores.get("COLLAB_DATA", "x", tmp_path / "x", deadline(clock, 0))
    assert raised.value.reason is Reason.DEADLINE_EXCEEDED


async def test_other_jetstream_errors_are_transient(
    stores: ObjectStores, fake_nats: FakeNats, clock: FakeClock, tmp_path: Path
) -> None:
    async def broken(bucket: str) -> None:
        raise nats.js.errors.NoStreamResponseError

    fake_nats.jetstream().object_store = broken
    with pytest.raises(TransientFailure) as raised:
        await stores.get("COLLAB_DATA", "x", tmp_path / "x", deadline(clock))
    assert raised.value.reason is Reason.OBJECT_STORE_UNAVAILABLE
