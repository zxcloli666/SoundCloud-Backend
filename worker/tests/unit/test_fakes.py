from __future__ import annotations

import asyncio
import json
from collections.abc import Mapping
from pathlib import Path

import aiohttp
import nats.errors
import nats.js.errors
import numpy as np
import pytest
from nats.js import api

from tests.fakes.clock import FakeClock
from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from tests.fakes.jetstream import PERMISSIONS_VIOLATION, FakeNats, ForbiddenCall
from tests.fakes.llm import FakeProvider, FakeProviderError, FakeRefiner
from tests.fakes.object_store import FakeBlobStore
from worker.contract import Contract
from worker.domain.deadline import Deadline
from worker.domain.outcome import PermanentFailure, Reason, TransientFailure
from worker.domain.ports import EngineUnavailable

AUDIO_TASK = {"sc_track_id": "1", "s3_url": "https://s3/x", "upload_generation": 1, "attempt": 1}


async def bound(fake_nats: FakeNats, contract: Contract, lane: str = "audio"):
    await fake_nats.connect()
    spec = contract.lane(lane)
    return await fake_nats.jetstream().pull_subscribe_bind(spec.durable, spec.stream)


async def test_fetch_ack_removes_from_work_queue(fake_nats: FakeNats, contract: Contract) -> None:
    subscription = await bound(fake_nats, contract)
    seq = fake_nats.enqueue("index.audio.new", AUDIO_TASK, {"Nats-Msg-Id": "storage-audio:1:1:1"})
    [message] = await subscription.fetch(batch=5, timeout=5)
    assert message.metadata.sequence.stream == seq
    assert message.metadata.num_delivered == 1
    assert message.headers == {"Nats-Msg-Id": "storage-audio:1:1:1"}
    assert json.loads(message.data) == AUDIO_TASK
    await message.ack_sync(timeout=5)
    assert seq not in fake_nats.streams["INDEX_AUDIO"].messages
    assert fake_nats.acks("ack_sync")[0].seq == seq
    with pytest.raises(nats.errors.MsgAlreadyAckdError):
        await message.ack()
    with pytest.raises(nats.js.errors.FetchTimeoutError):
        await subscription.fetch(batch=1, timeout=1)


async def test_ack_wait_expiry_redelivers_with_incremented_count(
    fake_nats: FakeNats, contract: Contract, clock: FakeClock
) -> None:
    subscription = await bound(fake_nats, contract)
    fake_nats.enqueue("index.audio.new", AUDIO_TASK)
    [first] = await subscription.fetch(batch=1, timeout=1)
    await clock.tick(59)
    with pytest.raises(nats.js.errors.FetchTimeoutError):
        await subscription.fetch(batch=1, timeout=0.5)
    await clock.tick(1)
    [second] = await subscription.fetch(batch=1, timeout=1)
    assert second.metadata.sequence.stream == first.metadata.sequence.stream
    assert second.metadata.num_delivered == 2


async def test_in_progress_resets_ack_wait_and_nak_delays(
    fake_nats: FakeNats, contract: Contract, clock: FakeClock
) -> None:
    subscription = await bound(fake_nats, contract)
    fake_nats.enqueue("index.audio.new", AUDIO_TASK)
    [message] = await subscription.fetch(batch=1, timeout=1)
    await clock.tick(50)
    await message.in_progress()
    await clock.tick(50)
    with pytest.raises(nats.js.errors.FetchTimeoutError):
        await subscription.fetch(batch=1, timeout=0.5)
    await message.nak(delay=30)
    await clock.tick(29)
    with pytest.raises(nats.js.errors.FetchTimeoutError):
        await subscription.fetch(batch=1, timeout=0.5)
    await clock.tick(1)
    [again] = await subscription.fetch(batch=1, timeout=1)
    assert again.metadata.num_delivered == 2
    assert [sent.kind for sent in fake_nats.sent] == ["in_progress", "nak"]
    assert fake_nats.sent[1].delay == 30


async def test_max_deliver_exhaustion_leaves_message_in_stream(
    fake_nats: FakeNats, contract: Contract, clock: FakeClock
) -> None:
    subscription = await bound(fake_nats, contract, "encode")
    seq = fake_nats.enqueue("encode.text.new", {"model": "mulan", "text": "x", "hash": "a" * 64})
    for delivery in range(1, 6):
        [message] = await subscription.fetch(batch=1, timeout=1)
        assert message.metadata.num_delivered == delivery
        await clock.tick(30)
    with pytest.raises(nats.js.errors.FetchTimeoutError):
        await subscription.fetch(batch=1, timeout=1)
    consumer = fake_nats.consumers[("ENCODE", "encode-workers")]
    assert consumer.advisories[0]["stream_seq"] == seq
    assert seq in fake_nats.streams["ENCODE"].messages


async def test_disconnect_buffers_acks_and_fails_ack_sync(
    fake_nats: FakeNats, contract: Contract, clock: FakeClock
) -> None:
    events: list[str] = []

    async def disconnected() -> None:
        events.append("disconnected")

    async def reconnected() -> None:
        events.append("reconnected")

    await fake_nats.connect(disconnected_cb=disconnected, reconnected_cb=reconnected)
    spec = contract.lane("audio")
    subscription = await fake_nats.jetstream().pull_subscribe_bind(spec.durable, spec.stream)
    fake_nats.enqueue("index.audio.new", AUDIO_TASK)
    [message] = await subscription.fetch(batch=1, timeout=1)
    await fake_nats.disconnect()
    await message.in_progress()
    assert fake_nats.sent == []
    with pytest.raises(nats.errors.TimeoutError):
        await message.ack_sync(timeout=2)
    with pytest.raises(nats.errors.TimeoutError):
        await subscription.fetch(batch=1, timeout=1)
    with pytest.raises(nats.errors.TimeoutError):
        await fake_nats.jetstream().publish("done.index_audio", b"{}")
    await fake_nats.reconnect()
    assert [sent.kind for sent in fake_nats.sent] == ["in_progress"]
    assert events == ["disconnected", "reconnected"]
    assert fake_nats.reconnects == 1


async def test_publish_dedups_by_msg_id_and_can_lose_pubacks(
    fake_nats: FakeNats, contract: Contract, clock: FakeClock
) -> None:
    await fake_nats.connect()
    js = fake_nats.jetstream()
    headers = {api.Header.MSG_ID: "done.audio:index_audio:1:1:1:7:ok"}
    first = await js.publish("done.index_audio", b"{}", headers=headers)
    second = await js.publish("done.index_audio", b"{}", headers=headers)
    assert first.duplicate is False and second.duplicate is True and first.seq == second.seq
    fake_nats.lost_pubacks = 1
    with pytest.raises(nats.errors.TimeoutError):
        await js.publish("done.index_audio", b"{}", headers={api.Header.MSG_ID: "lost"})
    assert len(fake_nats.streams["PIPELINE_DONE"].messages) == 2
    fake_nats.publish_faults.append(nats.js.errors.NoStreamResponseError())
    with pytest.raises(nats.js.errors.NoStreamResponseError):
        await js.publish("done.index_audio", b"{}")
    fake_nats.max_payload = 10
    with pytest.raises(nats.errors.MaxPayloadError):
        await js.publish("done.index_audio", b"x" * 11)
    with pytest.raises(nats.js.errors.NoStreamResponseError):
        await js.publish("nowhere.at.all", b"{}")


async def test_consumer_info_faults(fake_nats: FakeNats, contract: Contract) -> None:
    seen: list[Exception] = []

    async def error_cb(error: Exception) -> None:
        seen.append(error)

    await fake_nats.connect(error_cb=error_cb)
    js = fake_nats.jetstream()
    info = await js.consumer_info("INDEX_AUDIO", "audio-workers")
    assert info.config.max_deliver == 5 and info.config.ack_wait == 60
    with pytest.raises(nats.js.errors.NotFoundError):
        await js.consumer_info("INDEX_AUDIO", "ghost-workers")
    fake_nats.consumer_faults["audio-workers"] = "permissions"
    with pytest.raises(nats.errors.TimeoutError):
        await js.consumer_info("INDEX_AUDIO", "audio-workers", timeout=1)
    assert PERMISSIONS_VIOLATION in str(fake_nats.errors[0])
    assert seen == fake_nats.errors
    fake_nats.consumer_faults["audio-workers"] = nats.js.errors.ServiceUnavailableError(code=503)
    with pytest.raises(nats.js.errors.ServiceUnavailableError):
        await js.consumer_info("INDEX_AUDIO", "audio-workers")
    with pytest.raises(nats.js.errors.NotFoundError):
        await js.pull_subscribe_bind("ghost-workers", "INDEX_AUDIO")
    with pytest.raises(ForbiddenCall):
        await js.pull_subscribe("index.audio.new", durable="audio-workers")
    with pytest.raises(ForbiddenCall):
        await js.add_consumer("INDEX_AUDIO")


async def test_lingering_messages_are_drained_before_a_new_pull(
    fake_nats: FakeNats, contract: Contract
) -> None:
    subscription = await bound(fake_nats, contract)
    seq = fake_nats.enqueue("index.audio.new", AUDIO_TASK)
    lingering = fake_nats.linger(subscription, seq)
    assert subscription.pending_msgs == 1
    [message] = await subscription.fetch(batch=8, timeout=1)
    assert message is lingering
    assert subscription.fetches == [(8, 1, None)]


async def test_core_publish_subscribe_request(fake_nats: FakeNats) -> None:
    await fake_nats.connect()
    received: list[bytes] = []

    async def on_message(message) -> None:
        received.append(message.data)

    await fake_nats.subscribe("worker.health.*", cb=on_message)
    assert await fake_nats.deliver_core("worker.health.gpu-main", b"ping") == 1
    assert received == [b"ping"]
    await fake_nats.publish("worker.status.gpu-main", b"{}", headers={"X-Worker-Id": "gpu-main"})
    assert fake_nats.core_published[-1] == (
        "worker.status.gpu-main",
        b"{}",
        {"X-Worker-Id": "gpu-main"},
    )
    with pytest.raises(nats.errors.NoRespondersError):
        await fake_nats.request("nobody.home", b"")
    fake_nats.responders["echo"] = lambda payload: payload.upper()
    reply = await fake_nats.request("echo", b"hi")
    assert reply.data == b"HI"
    await fake_nats.flush()
    await fake_nats.close()
    with pytest.raises(nats.errors.ConnectionClosedError):
        await fake_nats.publish("x", b"")


async def test_object_store_fake(fake_nats: FakeNats) -> None:
    await fake_nats.connect()
    store = await fake_nats.jetstream().object_store("COLLAB_DATA")
    await store.put("collab-input-1", b'{"version":2}')
    result = await store.get("collab-input-1")
    assert result.data == b'{"version":2}' and result.info.size == 13
    with pytest.raises(nats.js.errors.ObjectNotFoundError):
        await store.get("missing")
    store.unavailable = True
    with pytest.raises(nats.js.errors.ServiceUnavailableError):
        await store.get("collab-input-1")
    with pytest.raises(nats.js.errors.BucketNotFoundError):
        await fake_nats.jetstream().object_store("NOPE")


async def test_blob_store_fake(tmp_path: Path) -> None:
    blobs = FakeBlobStore()
    blobs.add("COLLAB_DATA", "in", b"data")
    deadline = Deadline.after(10)
    target = tmp_path / "in.json"
    assert await blobs.get("COLLAB_DATA", "in", target, deadline) == 4
    assert target.read_bytes() == b"data"
    with pytest.raises(PermanentFailure) as missing:
        await blobs.get("COLLAB_DATA", "nope", target, deadline)
    assert missing.value.reason is Reason.OBJECT_NOT_FOUND
    await blobs.put("COLLAB_DATA", "out", target, deadline)
    assert blobs.buckets["COLLAB_DATA"]["out"] == b"data"
    blobs.unavailable = True
    with pytest.raises(TransientFailure) as down:
        await blobs.put("COLLAB_DATA", "out", target, deadline)
    assert down.value.reason is Reason.OBJECT_STORE_UNAVAILABLE


async def test_engines_fake_is_deterministic_and_scriptable(tmp_path: Path) -> None:
    engines = FakeEngines()
    deadline = Deadline.after(10)
    windows = np.zeros((3, 24_000 * 30), dtype=np.float32)
    windows[1] += 0.5
    first = await engines.embed_audio(windows, deadline)
    second = await engines.embed_audio(windows, deadline)
    assert first.mert.shape == (3, 1024) and first.clap.shape == (3, 512)
    assert np.allclose(first.mert, second.mert)
    assert not np.allclose(first.mert[0], first.mert[1])
    assert np.allclose(np.linalg.norm(first.mert, axis=1), 1.0)
    text = await engines.embed_text(["la", "lo"], "document", deadline)
    query = await engines.embed_text(["la"], "query", deadline)
    assert text.shape == (2, 1024) and not np.allclose(text[0], query[0])
    assert (await engines.embed_text_mulan(["x"], deadline)).shape == (1, 512)
    regions = await engines.vad(
        np.zeros(16_000 * 30, dtype=np.float32),
        threshold=0.45,
        min_speech_ms=250,
        min_silence_ms=400,
        pad_ms=200,
        deadline=deadline,
    )
    assert [region.start_s for region in regions] == [0.5, 9.0]
    alignment = await engines.align(
        np.zeros(16_000 * 4, dtype=np.float32), ["a", "b"], "en", deadline
    )
    assert len(alignment.spans) == 2 and alignment.spans[1].start_s == 2.0
    guesses = await engines.detect_language(["hello"], deadline)
    assert guesses[0][0].code == "en"
    training = await engines.train_collab(
        tmp_path / "in",
        tmp_path / "out",
        min_count=1,
        window=5,
        epochs=1,
        negative=5,
        deadline=deadline,
    )
    assert training.points_count == 300 and (tmp_path / "out").exists()
    engines.generated = '{"primary_artist": "x"}'
    assert await engines.generate("p", {}, 64, deadline) == '{"primary_artist": "x"}'
    engines.fail("separate", EngineUnavailable("sep", "broken"))
    with pytest.raises(EngineUnavailable):
        await engines.separate(np.zeros((2, 10), dtype=np.float32), deadline)
    assert [name for name, _ in engines.calls][-1] == "separate"
    expired = Deadline(at=0.0)
    with pytest.raises(TransientFailure):
        await engines.fingerprint(np.zeros(10, dtype=np.int16), 44_100, 1, expired)


async def test_audio_server_fake() -> None:
    async with FakeAudioServer(body=b"RIFF" * 100, slow_chunk_delay_s=0.01) as server:
        async with aiohttp.ClientSession() as session:
            async with session.get(server.url("/ok.wav")) as response:
                assert response.status == 200 and await response.read() == b"RIFF" * 100
            for path, status in (
                ("/missing", 404),
                ("/gone", 410),
                ("/forbidden", 403),
                ("/unauthorized", 401),
                ("/error", 500),
            ):
                async with session.get(server.url(path)) as response:
                    assert response.status == status
            async with session.get(server.url("/slow")) as response:
                assert await response.read() == b"RIFF" * 100
            async with session.get(server.url("/garbage")) as response:
                assert response.content_type == "audio/mpeg"
        assert server.requests[0] == "/ok.wav"


async def test_llm_provider_fake(clock: FakeClock) -> None:
    provider = FakeProvider("anthropic", clock)
    provider.reply_with({"primary_artist": "x"}, delay_s=2)
    provider.fail_with(RuntimeError("boom"))
    provider.hang()
    deadline = Deadline.after(5, clock.now)
    task = asyncio.create_task(provider.complete("prompt", {}, deadline))
    await clock.tick(0)
    await clock.tick(1)
    assert not task.done()
    await clock.tick(1)
    assert await task == {"primary_artist": "x"}
    with pytest.raises(RuntimeError):
        await provider.complete("prompt", {}, deadline)
    hung = asyncio.create_task(provider.complete("prompt", {}, deadline))
    await clock.tick(0)
    await clock.tick(1)
    hung.cancel()
    with pytest.raises(asyncio.CancelledError):
        await hung
    assert provider.cancelled == 1
    with pytest.raises(FakeProviderError):
        await provider.complete("prompt", {}, deadline)
    assert len(provider.calls) == 4


async def test_refiner_fake_applies_grounding(clock: FakeClock) -> None:
    refiner = FakeRefiner()
    refiner.reply_with({"primary_artist": "Kendrick Lamar"})
    refiner.reply_with({"primary_artist": "Invented"})
    deadline = Deadline.after(5, clock.now)

    def grounded(reply: Mapping[str, object]) -> bool:
        return reply["primary_artist"] == "Kendrick Lamar"

    assert await refiner.refine("p", {}, grounded, deadline) == {"primary_artist": "Kendrick Lamar"}
    assert await refiner.refine("p", {}, grounded, deadline) is None
    assert await refiner.refine("p", {}, grounded, deadline) is None
    assert refiner.ungrounded == 1 and len(refiner.calls) == 3
    clock.advance(6)
    with pytest.raises(TransientFailure):
        await refiner.refine("p", {}, grounded, deadline)
