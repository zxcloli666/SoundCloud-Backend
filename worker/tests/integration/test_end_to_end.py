from __future__ import annotations

import asyncio
import hashlib
import io
import json
import os
import signal
import socket
import sys
import time
import uuid
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

import nats
import numpy as np
import psutil
import pytest
import soundfile as sf
import soxr
from aiohttp import web
from nats.aio.client import Client
from nats.aio.msg import Msg
from nats.aio.subscription import Subscription
from nats.js import JetStreamContext
from nats.js.manager import JetStreamManager

from tests.integration.provision import consumer_config, provision_like_jobs, reset
from worker.contract import Contract

pytestmark = pytest.mark.integration

WORKER_ROOT = Path(__file__).resolve().parents[2]
CONFIG_DIR = WORKER_ROOT / "config"
CONTRACT_PATH = WORKER_ROOT / "contract" / "worker-contract.json"
WORKER_ID = "e2e-worker"
ALL_LANES = ("audio", "lyrics", "transcribe", "encode", "collab", "ai")
CPU_LANES = ("collab", "ai")
SPEECH_CLIP_ENV = "WORKER_TEST_SPEECH_CLIP"
MARKER_ENV = "WORKER_E2E_MARKER"
TRANSCRIPT = "Mr Quilter is the apostle of the middle classes and we are glad to welcome his gospel"
ENCODE_TEXT = "a quiet song about the sea"
CLIP_RATE = 16_000
SILENCE_S = 1.0
LLM_ARTIST = "M83"
RPC_WINDOW_S = 20.0
READY_TIMEOUT_S = 600.0
DONE_TIMEOUT_S = 600.0
RPC_TIMEOUT_S = 30.0
STOP_TIMEOUT_S = 90.0
DRIFT_TIMEOUT_S = 150.0
COLLAB_OBJECT = "collab-e2e-input"
COLLAB_ITEMS = 60
COLLAB_SESSION_LENGTH = 8
QUICK_COLLAB = (600, 20)
SLOW_COLLAB = (100_000, 50)
SHORT_GRACE_S = "2"
SLOW_LLM_S = 10.0
QUEUE_TASKS: Mapping[str, tuple[str, str]] = {
    "audio": ("index.audio.new", "storage-audio:901:1:1"),
    "lyrics": ("embed.lyrics.new", "embed:902:lyr:902:1"),
    "transcribe": ("transcribe.audio.new", "transcribe:903:1:1"),
    "encode-lyrics": ("encode.text.new", "encode:lyrics"),
    "encode-mulan": ("encode.text.new", "encode:mulan"),
    "collab": ("train.collab.new", "collab:e2e"),
}
REQUIRED_SLOTS = ("muq", "mulan", "text", "sep", "asr", "align", "mms", "vad", "lid", "fingerprint")


@dataclass
class Live:
    url: str
    admin: Client
    js: JetStreamContext
    jsm: JetStreamManager
    contract: Contract

    async def publish(
        self, subject: str, payload: Mapping[str, object], headers: Mapping[str, str]
    ) -> None:
        assert self.contract.validate(subject, payload) == [], (subject, payload)
        await self.js.publish(subject, json.dumps(payload).encode(), headers=dict(headers))

    async def done_messages(self) -> list[tuple[str, dict[str, object]]]:
        info = await self.jsm.stream_info("PIPELINE_DONE")
        if info.state.messages == 0:
            return []
        messages = []
        for seq in range(info.state.first_seq, info.state.last_seq + 1):
            raw = await self.jsm.get_msg("PIPELINE_DONE", seq)
            messages.append((raw.subject or "", json.loads(raw.data or b"{}")))
        return messages

    async def messages_left(self, stream: str) -> int:
        return int((await self.jsm.stream_info(stream)).state.messages)


@dataclass
class Fixtures:
    base_url: str = ""
    clip: bytes = b""
    llm_delay_s: float = 0.0
    llm_requests: list[dict[str, object]] = field(default_factory=list)

    @property
    def clip_url(self) -> str:
        return f"{self.base_url}/clip.flac"

    @property
    def llm_url(self) -> str:
        return f"{self.base_url}/v1/chat/completions"


@dataclass
class Serve:
    process: asyncio.subprocess.Process
    log_path: Path
    marker: str
    statuses: list[dict[str, object]]

    def events(self) -> list[dict[str, object]]:
        lines = self.log_path.read_text(encoding="utf-8", errors="replace").splitlines()
        return [json.loads(line) for line in lines if line.startswith("{")]

    def event_names(self) -> set[str]:
        return {str(event.get("event")) for event in self.events()}

    def tail(self) -> str:
        return "\n".join(self.log_path.read_text(errors="replace").splitlines()[-40:])

    async def wait_exit(self, timeout_s: float) -> int:
        return await asyncio.wait_for(self.process.wait(), timeout_s)

    async def terminate(self) -> int:
        if self.process.returncode is None:
            self.process.send_signal(signal.SIGTERM)
        return await self.wait_exit(STOP_TIMEOUT_S)


@pytest.fixture
async def live(contract: Contract) -> AsyncIterator[Live]:
    url = os.environ["NATS_TEST_URL"]
    admin = await nats.connect(url)
    jsm = admin.jsm()
    js = admin.jetstream()
    await reset(jsm, contract)
    await provision_like_jobs(jsm, js, contract)
    yield Live(url, admin, js, jsm, contract)
    await reset(jsm, contract)
    await admin.close()


@pytest.fixture
async def fixtures() -> AsyncIterator[Fixtures]:
    state = Fixtures()

    async def serve_clip(request: web.Request) -> web.Response:
        return web.Response(body=state.clip, content_type="audio/flac")

    async def complete(request: web.Request) -> web.Response:
        state.llm_requests.append(await request.json())
        await asyncio.sleep(state.llm_delay_s)
        return web.json_response(openai_reply(llm_answer()))

    app = web.Application()
    app.router.add_get("/clip.flac", serve_clip)
    app.router.add_post("/v1/chat/completions", complete)
    runner = web.AppRunner(app)
    await runner.setup()
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    await web.SockSite(runner, sock).start()
    state.base_url = f"http://127.0.0.1:{sock.getsockname()[1]}"
    yield state
    await runner.cleanup()


@pytest.mark.models
async def test_every_lane_is_served_end_to_end(
    live: Live, fixtures: Fixtures, tmp_path: Path
) -> None:
    fixtures.clip = speech_clip()
    worker = await start_serve(live, fixtures, tmp_path, ALL_LANES)
    try:
        status = await wait_for_status(worker, all_lanes_serving(ALL_LANES))
        await put_collab_input(live, *QUICK_COLLAB)
        for name, (subject, msg_id) in QUEUE_TASKS.items():
            await live.publish(subject, queue_task(name, fixtures), {"Nats-Msg-Id": msg_id})
        replies = {
            method: await (await send_rpc(live, method, request)).next_msg(RPC_TIMEOUT_S)
            for method, request in rpc_requests().items()
        }
        dones = await wait_for_dones(live, worker, len(QUEUE_TASKS))
        health_reply = await live.admin.request(f"worker.health.{WORKER_ID}", b"", timeout=5)
        health_exit = await run_health(tmp_path / "health.json")
    finally:
        code = await worker.terminate()

    assert code == 0, worker.tail()
    assert_dones(live.contract, dones)
    assert_rpc_replies(live.contract, replies, fixtures)
    assert_status(status)
    assert json.loads(health_reply.data)["worker_id"] == WORKER_ID
    assert health_exit == 0
    assert await collab_vectors_exist(live)
    events = worker.event_names()
    assert {"shutdown_started", "supervisor_stopped", "worker_stopped"} <= events
    assert "background_task_crashed" not in events
    assert_nothing_left(worker)


async def test_sigterm_mid_task_naks_the_queue_task_and_expires_the_rpc(
    live: Live, fixtures: Fixtures, tmp_path: Path
) -> None:
    fixtures.llm_delay_s = SLOW_LLM_S
    worker = await start_serve(
        live, fixtures, tmp_path, CPU_LANES, WORKER__RUNTIME__SHUTDOWN_GRACE_S=SHORT_GRACE_S
    )
    try:
        await wait_for_status(worker, all_lanes_serving(CPU_LANES))
        await put_collab_input(live, *SLOW_COLLAB)
        await live.publish(
            "train.collab.new", collab_task(SLOW_COLLAB[1]), {"Nats-Msg-Id": "collab:slow"}
        )
        pending = await send_rpc(live, "resolve_artist", rpc_requests()["resolve_artist"])
        await wait_until(worker, lambda: both_in_flight(live, fixtures), READY_TIMEOUT_S, "work")
        code = await worker.terminate()
    finally:
        if worker.process.returncode is None:
            await worker.terminate()

    assert code == 0, worker.tail()
    reply = json.loads((await pending.next_msg(RPC_TIMEOUT_S)).data)
    assert reply == {"ok": False, "error": "expired"}
    assert await live.done_messages() == []
    assert await live.messages_left("TRAIN_COLLAB") == 1
    events = worker.event_names()
    assert {"lane_aborting_tasks", "shutdown_started", "worker_stopped"} <= events
    assert_nothing_left(worker)


async def test_consumer_drift_at_start_exits_78_before_any_engine(
    live: Live, fixtures: Fixtures, tmp_path: Path
) -> None:
    lane = live.contract.lane("ai")
    await live.jsm.add_consumer(lane.stream, consumer_config(lane, ack_wait=61.0))
    worker = await start_serve(live, fixtures, tmp_path, CPU_LANES)

    code = await worker.wait_exit(60)

    assert code == 78, worker.tail()
    events = worker.events()
    [drift] = [event for event in events if event.get("event") == "consumer_drift_at_start"]
    assert drift["lane"] == "ai" and drift["diff"] == {"ack_wait_s": [30.0, 61.0]}
    assert "engine_spawned" not in {event.get("event") for event in events}


async def test_consumer_drift_in_work_drains_and_exits_78(
    live: Live, fixtures: Fixtures, tmp_path: Path
) -> None:
    worker = await start_serve(live, fixtures, tmp_path, CPU_LANES)
    try:
        await wait_for_status(worker, all_lanes_serving(CPU_LANES))
        lane = live.contract.lane("ai")
        await live.jsm.add_consumer(lane.stream, consumer_config(lane, ack_wait=61.0))
        code = await worker.wait_exit(DRIFT_TIMEOUT_S)
    finally:
        if worker.process.returncode is None:
            await worker.terminate()

    assert code == 78, worker.tail()
    assert {"consumer_drift_in_work", "shutdown_started", "worker_stopped"} <= worker.event_names()
    assert_nothing_left(worker)


async def start_serve(
    live: Live, fixtures: Fixtures, tmp_path: Path, lanes: Sequence[str], **overrides: str
) -> Serve:
    marker = uuid.uuid4().hex
    statuses: list[dict[str, object]] = []

    async def on_status(msg: Msg) -> None:
        statuses.append(json.loads(msg.data))

    await live.admin.subscribe(f"worker.status.{WORKER_ID}", cb=on_status)
    log_path = tmp_path / "serve.log"
    env = {**serve_env(live.url, fixtures, tmp_path, lanes, marker), **overrides}
    with log_path.open("wb") as log_file:
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            "-m",
            "worker",
            "serve",
            "--config-dir",
            str(CONFIG_DIR),
            "--health-file",
            str(tmp_path / "health.json"),
            cwd=WORKER_ROOT,
            env=env,
            stdout=log_file,
            stderr=asyncio.subprocess.STDOUT,
        )
    return Serve(process, log_path, marker, statuses)


def serve_env(
    url: str, fixtures: Fixtures, tmp_path: Path, lanes: Sequence[str], marker: str
) -> dict[str, str]:
    parts = urlsplit(url)
    bare = urlunsplit((parts.scheme, f"{parts.hostname}:{parts.port or 4222}", "", "", ""))
    inherited = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("WORKER__")
        and name not in ("WORKER_PROFILE", "ANTHROPIC_API_KEY", "CUDA_VISIBLE_DEVICES")
    }
    return {
        **inherited,
        "WORKER_NODE_NAME": WORKER_ID,
        "NATS_URL": bare,
        "NATS_USER": parts.username or "",
        "NATS_PASSWORD": parts.password or "",
        "WORKER__WORKER__BUILD": "e2e",
        "WORKER__WORKER__WORK_DIR": str(tmp_path / "work"),
        "WORKER__WORKER__CONTRACT": str(CONTRACT_PATH),
        "WORKER__RUNTIME__MODE": "lane",
        "WORKER__LANES__ENABLED": json.dumps(list(lanes)),
        "WORKER__SLOTS__SEP__REPLICAS": "1",
        "WORKER__SLOTS__ASR__REPLICAS": "1",
        "WORKER__SLOTS__ALIGN__REPLICAS": "1",
        "LLM_FALLBACK_URL": fixtures.llm_url,
        "LLM_FALLBACK_MODEL": "fake-llm",
        "LLM_FALLBACK_KEY": "fake-key",
        "HF_HUB_OFFLINE": "1",
        "TRANSFORMERS_OFFLINE": "1",
        "TOKENIZERS_PARALLELISM": "false",
        "FASTTEXT_HOME": os.environ.get("FASTTEXT_HOME", str(Path.home() / ".cache" / "fasttext")),
        "PYTHONUNBUFFERED": "1",
        MARKER_ENV: marker,
    }


def speech_clip() -> bytes:
    path = os.environ.get(SPEECH_CLIP_ENV)
    if not path or not Path(path).is_file():
        pytest.skip(f"needs a LibriSpeech clip in {SPEECH_CLIP_ENV}")
    audio, rate = sf.read(path, dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    if rate != CLIP_RATE:
        audio = soxr.resample(audio, rate, CLIP_RATE, quality="HQ")
    silence = np.zeros(int(SILENCE_S * CLIP_RATE), dtype=np.float32)
    samples = np.concatenate([silence, audio, silence, audio, silence]).astype(np.float32)
    buffer = io.BytesIO()
    sf.write(buffer, samples, CLIP_RATE, format="FLAC")
    return buffer.getvalue()


def queue_task(name: str, fixtures: Fixtures) -> dict[str, object]:
    lyrics = f"{TRANSCRIPT}\n{TRANSCRIPT}"
    tasks: dict[str, dict[str, object]] = {
        "audio": {
            "sc_track_id": "901",
            "s3_url": fixtures.clip_url,
            "upload_generation": 1,
            "attempt": 1,
        },
        "lyrics": {
            "sc_track_id": "902",
            "request_id": "lyr:902:1",
            "text": lyrics,
            "language": None,
        },
        "transcribe": {
            "sc_track_id": "903",
            "upload_generation": 1,
            "attempt": 1,
            "audio_url": fixtures.clip_url,
            "reference_text": lyrics,
            "reference_lines_total": 2,
            "language": "en",
            "mode": "align",
        },
        "encode-lyrics": {"model": "lyrics", "text": ENCODE_TEXT, "hash": sha256(ENCODE_TEXT)},
        "encode-mulan": {"model": "mulan", "text": ENCODE_TEXT, "hash": sha256(ENCODE_TEXT)},
        "collab": collab_task(QUICK_COLLAB[1]),
    }
    return tasks[name]


def collab_task(epochs: int) -> dict[str, object]:
    return {
        "object": COLLAB_OBJECT,
        "dataset_version": 2,
        "dim": 128,
        "epochs": epochs,
        "min_count": 1,
        "negative": 5,
        "window": 3,
    }


def rpc_requests() -> dict[str, dict[str, object]]:
    return {
        "resolve_artist": {"title": "Midnight City", "uploader": LLM_ARTIST},
        "match_track": {
            "target": {"artist": LLM_ARTIST, "title": "Midnight City"},
            "candidates": [
                {"id": 7, "artist": LLM_ARTIST, "title": "Midnight City", "duration_sec": 243.0},
                {"id": 8, "artist": "Someone Else", "title": "Another Song"},
            ],
        },
    }


def llm_answer() -> dict[str, object]:
    return {
        "primary_artist": LLM_ARTIST,
        "featured": [],
        "producers": [],
        "remixers": [],
        "album": None,
        "confidence": 0.95,
    }


def openai_reply(answer: Mapping[str, object]) -> dict[str, object]:
    message = {"role": "assistant", "content": json.dumps(answer)}
    return {"choices": [{"finish_reason": "stop", "message": message}]}


async def send_rpc(live: Live, method: str, request: Mapping[str, object]) -> Subscription:
    inbox = f"_INBOX.e2e.{uuid.uuid4().hex}"
    subscription = await live.admin.subscribe(inbox, max_msgs=1)
    headers = {
        "Nats-Msg-Id": f"rpc:{method}:{uuid.uuid4().hex}",
        "X-Reply-To": inbox,
        "X-Deadline": str(int((time.time() + RPC_WINDOW_S) * 1000)),
    }
    await live.publish(f"ai.rpc.{method}", request, headers)
    return subscription


async def put_collab_input(live: Live, sessions: int, epochs: int) -> None:
    generator = np.random.default_rng(epochs)
    rows = [
        [1000 + (int(start) + step) % COLLAB_ITEMS for step in range(COLLAB_SESSION_LENGTH)]
        for start in generator.integers(0, COLLAB_ITEMS, sessions)
    ]
    store = await live.js.object_store("COLLAB_DATA")
    await store.put(COLLAB_OBJECT, json.dumps({"version": 2, "sessions": rows}).encode())


async def collab_vectors_exist(live: Live) -> bool:
    store = await live.js.object_store("COLLAB_DATA")
    info = await store.get_info(f"{COLLAB_OBJECT}-vectors")
    return info.size > 0


def all_lanes_serving(lanes: Sequence[str]) -> Callable[[Mapping[str, object]], bool]:
    def serving(status: Mapping[str, object]) -> bool:
        states = status.get("lanes")
        if not isinstance(states, Mapping):
            return False
        return all(
            isinstance(states.get(lane), Mapping) and states[lane].get("state") == "serving"
            for lane in lanes
        )

    return serving


async def both_in_flight(live: Live, fixtures: Fixtures) -> bool:
    lane = live.contract.lane("collab")
    info = await live.jsm.consumer_info(lane.stream, lane.durable)
    return info.num_ack_pending == 1 and bool(fixtures.llm_requests)


async def wait_for_status(
    worker: Serve, ready: Callable[[Mapping[str, object]], bool]
) -> dict[str, object]:
    found: list[dict[str, object]] = []

    async def seen() -> bool:
        found.extend(status for status in worker.statuses if ready(status))
        return bool(found)

    await wait_until(worker, seen, READY_TIMEOUT_S, "every lane serving in worker.status")
    return found[-1]


async def wait_for_dones(
    live: Live, worker: Serve, count: int
) -> list[tuple[str, dict[str, object]]]:
    found: list[tuple[str, dict[str, object]]] = []

    async def enough() -> bool:
        found[:] = await live.done_messages()
        return len(found) >= count

    await wait_until(worker, enough, DONE_TIMEOUT_S, f"{count} done messages")
    return found


async def wait_until(
    worker: Serve, condition: Callable[[], Awaitable[bool]], timeout_s: float, what: str
) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if worker.process.returncode is not None:
            raise AssertionError(f"serve exited {worker.process.returncode}:\n{worker.tail()}")
        if await condition():
            return
        await asyncio.sleep(0.5)
    raise AssertionError(f"timed out waiting for {what}:\n{worker.tail()}")


async def run_health(path: Path) -> int:
    process = await asyncio.create_subprocess_exec(
        sys.executable, "-m", "worker", "health", "--path", str(path), cwd=WORKER_ROOT
    )
    return await asyncio.wait_for(process.wait(), 30)


def assert_dones(contract: Contract, dones: list[tuple[str, dict[str, object]]]) -> None:
    by_subject: dict[str, list[dict[str, object]]] = {}
    for subject, done in dones:
        assert contract.validate(subject, done) == [], (subject, done)
        assert done["producer"]["worker_id"] == WORKER_ID
        by_subject.setdefault(subject, []).append(done)
    [audio] = by_subject["done.index_audio"]
    assert audio["status"] == "ok", audio
    assert len(audio["mert"]) == 1024 and len(audio["clap"]) == 512
    assert isinstance(audio["fingerprint"], str) and audio["fingerprint"]
    [lyrics] = by_subject["done.embed_lyrics"]
    assert lyrics["status"] == "ok" and lyrics["language"] == "en", lyrics
    assert len(lyrics["vec"]) == 1024
    [transcribe] = by_subject["done.transcribe"]
    assert transcribe["status"] == "ok", transcribe
    assert transcribe["producer"]["sync_version"] == transcribe["sync_version"]
    assert transcribe["lines_total"] == 2 and transcribe["lines_unplaced"] == 0
    assert transcribe["synced_lrc"].count("Quilter") == 2
    encodes = {done["model"]: done for done in by_subject["done.encode"]}
    assert encodes["lyrics"]["status"] == "ok" and len(encodes["lyrics"]["vector"]) == 1024
    assert encodes["mulan"]["status"] == "ok" and len(encodes["mulan"]["vector"]) == 512
    [collab] = by_subject["done.train_collab"]
    assert collab["status"] == "ok" and collab["trained"] is True, collab
    assert collab["input_object"] == COLLAB_OBJECT
    assert collab["points_count"] == COLLAB_ITEMS


def assert_rpc_replies(contract: Contract, replies: Mapping[str, Msg], fixtures: Fixtures) -> None:
    answers = {method: json.loads(reply.data) for method, reply in replies.items()}
    for method, answer in answers.items():
        assert contract.validate(f"ai.rpc.{method}.reply", answer) == [], (method, answer)
        assert answer["ok"] is True, answer
    resolved = answers["resolve_artist"]["data"]
    assert resolved["primary_artist"] == LLM_ARTIST and resolved["source"] == "llm"
    assert [request["model"] for request in fixtures.llm_requests] == ["fake-llm"]
    assert answers["match_track"]["data"]["match_id"] == 7


def assert_status(status: Mapping[str, object]) -> None:
    assert status["worker_id"] == WORKER_ID and status["build"] == "e2e"
    assert status["nats"]["connected"] is True
    assert set(status["lanes"]) == set(ALL_LANES)
    for slot in REQUIRED_SLOTS:
        assert status["slots"][slot]["state"] == "ready", (slot, status["slots"][slot])
    gpu = status["gpu"]
    assert isinstance(gpu, Mapping) and gpu["total_mib"] > 0, gpu


def assert_nothing_left(worker: Serve) -> None:
    leftovers = sorted(Path("/dev/shm").glob(f"wk-{worker.process.pid}-*"))
    assert leftovers == [], leftovers
    engines = [
        process.info["pid"]
        for process in psutil.process_iter(["pid", "environ"])
        if (process.info["environ"] or {}).get(MARKER_ENV) == worker.marker
    ]
    assert engines == [], engines


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
