from __future__ import annotations

import asyncio
import logging
import os
import signal
import time
from collections.abc import Awaitable, Callable, Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

import aiohttp
import nats.errors
from nats.aio.client import Client

from worker import settings as settings_module
from worker.bus.connection import Connection
from worker.bus.consumers import EX_CONFIG, ConfigDrift, ConsumerWatch, served_lanes
from worker.bus.inflight import Inflight
from worker.bus.lane_runner import LaneRunner, Processor, QueueHandler
from worker.bus.lease import Leases, drop_if_stale
from worker.bus.objects import ObjectStores
from worker.bus.outbox import Outbox
from worker.bus.rpc import RpcHandler
from worker.bus.status import StatusReporter
from worker.contract import Contract, LaneSpec
from worker.domain.audio.source import AudioSource
from worker.domain.collab import CollabLane
from worker.domain.deadline import MonotonicClock
from worker.domain.embed_lyrics import EmbedLyricsLane
from worker.domain.encode_text import EncodeTextLane
from worker.domain.index_audio import IndexAudioLane
from worker.domain.lyrics.transcribe import TranscribeLane
from worker.domain.metadata.match import TrackMatcher
from worker.domain.metadata.resolve import ArtistResolver
from worker.domain.outcome import Producer
from worker.domain.taste import TasteLane
from worker.domain.workspace import Workspace
from worker.engines import BATCHED_SLOTS, EngineSlots, RuntimeEngines
from worker.fetch_models import FASTTEXT_HOME_ENV
from worker.llm.chain import build_refiner
from worker.observability import logging as json_logging
from worker.observability.counters import Counters
from worker.observability.health_file import HEALTH_PATH, LANES, SINCE, STATE, HealthFile
from worker.observability.logging import JsonLog
from worker.runtime.batcher import Batcher
from worker.runtime.engine_client import Launch
from worker.runtime.protocol import SlotSpec
from worker.runtime.supervisor import (
    MODE_SLOT,
    STATE_BROKEN,
    STATE_LOADING,
    EnginePlan,
    RuntimePolicy,
    Supervisor,
    plan_engines,
)
from worker.settings import CPU_ONLY_SLOTS, Settings, SettingsError

CPU_TOOLS = "cpu-tools"
CPU_TOOL_LOADERS: Mapping[str, str] = {
    "vad": "worker.models.vad:SileroVad",
    "lid": "worker.models.lid:LidSlot",
    "fingerprint": "worker.models.fingerprint:FingerprintSlot",
}
SLOT_LOADERS: Mapping[str, str] = {
    "muq": "worker.models.muq:MuqSlot",
    "mulan": "worker.models.mulan:MulanSlot",
    "text": "worker.models.text_embed:TextEmbedSlot",
    "sep": "worker.models.roformer:RoformerSeparator",
    "asr": "worker.models.qwen_asr:QwenAsr",
    "align": "worker.models.qwen_align:QwenAligner",
    "mms": "worker.models.mms_align:MmsAligner",
    "train-collab": "worker.models.item2vec:Item2VecSlot",
    "train-taste": "worker.models.taste_trainer:TasteTrainerSlot",
    settings_module.LOCAL_LLM_SLOT: "worker.models.local_llm:LocalLlmSlot",
}
LANE_GATES: Mapping[str, tuple[str, ...]] = {
    "audio": ("muq", "mulan"),
    "lyrics": ("text", "lid"),
    "transcribe": ("asr", "mms", "vad", "lid"),
    "encode": ("text", "mulan"),
    "collab": ("train-collab",),
    "taste": ("train-taste",),
    "ai": (),
}
PRODUCER_MODELS: Mapping[str, Mapping[str, str]] = {
    "audio": {"mert": "muq", "clap": "mulan"},
    "lyrics": {"lyrics": "text", "lid": CPU_TOOLS},
    "transcribe": {"sep": "sep", "asr": "asr", "align": "align", "mms": "mms", "lid": CPU_TOOLS},
    "encode": {"mulan": "mulan", "lyrics": "text"},
    "collab": {},
    "taste": {},
}
SLOW_LANES = frozenset({"audio", "transcribe"})
AUDIO_FETCH_LANES = frozenset({"audio", "transcribe"})
LLM_CALLS_PER_REQUEST = 2
POOL_SPARE = 8
EXIT_CRASHED = 1
GATE_INTERVAL_S = 5.0
OUTBOX_FLUSH_S = 10.0
LANE_TASK_PREFIX = "lane:"
GPU_PROBE_INTERVAL_S = 15.0
GPU_PROBE_TIMEOUT_S = 5.0
GPU_QUERY = (
    "nvidia-smi",
    "--query-gpu=name,memory.total,memory.used",
    "--format=csv,noheader,nounits",
)

log = logging.getLogger("worker.app")


class StartupError(Exception):
    pass


async def run(
    settings: Settings,
    contract: Contract,
    *,
    health_path: Path = HEALTH_PATH,
    environ: Mapping[str, str] = os.environ,
) -> int:
    json_log = JsonLog(worker_id=settings.worker.id)
    json_logging.install(json_log, logging.INFO)
    try:
        blueprint = Blueprint.of(settings, contract, environ)
    except (SettingsError, StartupError) as error:
        json_log.error("startup_config_error", error=str(error))
        return EX_CONFIG
    for gap in settings_module.fallback_gaps(settings):
        json_log.warning("fallback_disabled", gap=gap)
    node = Node(settings, contract, blueprint, health_path, json_log)
    return await node.serve()


@dataclass(frozen=True)
class Blueprint:
    lanes: tuple[LaneSpec, ...]
    plans: tuple[EnginePlan, ...]
    specs: Mapping[str, SlotSpec]
    launch: Launch
    policy: RuntimePolicy
    sync_version: str

    @classmethod
    def of(cls, settings: Settings, contract: Contract, environ: Mapping[str, str]) -> Blueprint:
        lanes = served_lanes(settings, contract)
        check_slow_lanes(settings)
        wanted = settings_module.slots_for_lanes(settings)
        specs = {name: slot_spec(settings, name) for name in wanted if name != CPU_TOOLS}
        replicas = {name: settings.slots[name].replicas for name in specs}
        plans = list(plan_engines(settings.runtime.mode, specs, replicas))
        env: dict[str, str] = {}
        if CPU_TOOLS in wanted:
            tools = cpu_tool_specs(settings)
            plans.append(EnginePlan(CPU_TOOLS, tools))
            specs.update({spec.name: spec for spec in tools})
            env[FASTTEXT_HOME_ENV] = required_env(environ, FASTTEXT_HOME_ENV)
        runtime = settings.runtime
        policy = RuntimePolicy(
            onednn=runtime.onednn,
            release_after_call=runtime.release_after_call,
            recycle_after_calls=runtime.recycle_after_calls,
            recycle_gap_mib=runtime.recycle_gap_mib,
            recycle_overlap=runtime.mode == MODE_SLOT,
            idle_unload_s=float(runtime.idle_unload_s),
            oom_unload=runtime.oom_unload,
        )
        return cls(
            lanes=lanes,
            plans=tuple(plans),
            specs=specs,
            launch=Launch(env=env),
            policy=policy,
            sync_version=settings_module.sync_version(settings),
        )


class Node:
    def __init__(
        self,
        settings: Settings,
        contract: Contract,
        blueprint: Blueprint,
        health_path: Path,
        json_log: JsonLog,
    ) -> None:
        self.settings = settings
        self.contract = contract
        self.blueprint = blueprint
        self.counters = Counters()
        self.clock = MonotonicClock()
        self.log = json_log
        self.connection = Connection(
            Client(), settings.nats, settings.worker.id, self.counters, self.clock
        )
        self.supervisor = Supervisor(
            blueprint.plans,
            blueprint.policy,
            self.counters,
            launch=blueprint.launch,
            log=json_log,
        )
        self.batchers = {
            name: Batcher(
                name, spec.max_batch, spec.max_wait_ms, self.supervisor, self.counters, log=json_log
            )
            for name, spec in blueprint.specs.items()
            if name in BATCHED_SLOTS
        }
        self.slots = EngineSlots(
            self.supervisor,
            self.batchers,
            {name: spec.max_batch for name, spec in blueprint.specs.items()},
            drop_if_stale,
        )
        self.workspace = Workspace(Path(settings.worker.work_dir), self.counters)
        self.health = HealthFile(health_path)
        self.gpu = GpuProbe(self.counters, enabled=settings.runtime.device != "cpu")
        self.lanes: dict[str, ServedLane] = {}
        self.stop_requested = asyncio.Event()
        self.crashed = False
        self.hurry = asyncio.Event()
        self.health_stop = asyncio.Event()
        self.background: list[asyncio.Task[None]] = []
        self.lane_states: dict[str, tuple[str, float]] = {}

    async def serve(self) -> int:
        self.install_signals()
        await self.connection.open()
        try:
            watches = self.watches()
            drift = await self.verify(watches)
            if drift is not None:
                return drift.exit_code
            removed = self.workspace.purge()
            self.log.info("workspace_purged", entries=removed)
            pools = HttpPools.of(self.blueprint.lanes, self.settings.lanes.capacity)
            async with pools.llm_session() as llm_session, pools.audio_session() as audio_session:
                await self.supervisor.start()
                try:
                    self.assemble(llm_session, audio_session, watches)
                    self.start_background()
                    await self.serve_health()
                    code = await self.until_stopped()
                    await self.shutdown(code)
                finally:
                    await self.stop_runtime()
            self.log.info("worker_stopped", exit_code=code)
            return code
        finally:
            await self.close_connection()

    def install_signals(self) -> None:
        loop = asyncio.get_running_loop()
        for number in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(number, self.on_signal, number)

    def on_signal(self, number: int) -> None:
        if self.stop_requested.is_set():
            self.log.warning("second_stop_signal", signal=signal.Signals(number).name)
            self.hurry.set()
            return
        self.log.info("stop_signal", signal=signal.Signals(number).name)
        self.stop_requested.set()

    def watches(self) -> dict[str, ConsumerWatch]:
        lanes = self.settings.lanes
        return {
            lane.name: ConsumerWatch(
                lane,
                self.connection.js,
                self.connection,
                self.counters,
                self.clock,
                lanes.required.get(lane.name, False),
                lanes.required_grace_s,
                public=self.settings.is_public,
            )
            for lane in self.blueprint.lanes
        }

    async def verify(self, watches: Mapping[str, ConsumerWatch]) -> ConfigDrift | None:
        for watch in watches.values():
            try:
                state = await watch.verify_at_start()
            except ConfigDrift as drift:
                self.log.error(
                    "consumer_drift_at_start",
                    lane=drift.lane,
                    diff={key: list(pair) for key, pair in drift.diff.items()},
                )
                return drift
            self.log.info("consumer_checked", lane=watch.lane.name, state=state.value)
        return None

    def assemble(
        self,
        llm_session: aiohttp.ClientSession,
        audio_session: aiohttp.ClientSession,
        watches: Mapping[str, ConsumerWatch],
    ) -> None:
        settings = self.settings
        js = self.connection.js
        worker = settings.worker
        self.outbox = Outbox(
            js,
            self.connection,
            settings.nats.outbox,
            self.counters,
            self.clock,
            worker.id,
            worker.build,
            self.contract.headers,
        )
        source = AudioSource(
            audio_session,
            timeout_s=settings.audio.download_timeout_s,
            max_bytes=settings.audio.max_download_mib << 20,
            counters=self.counters,
        )
        engines = RuntimeEngines(self.slots, priority=False)
        processors: dict[str, Callable[[], Processor]] = {
            "audio": lambda: IndexAudioLane(
                engines, source, self.workspace, settings.audio, self.counters
            ),
            "lyrics": lambda: EmbedLyricsLane(engines, self.counters),
            "transcribe": lambda: TranscribeLane(
                engines,
                source,
                self.workspace,
                settings.audio,
                settings.sync,
                self.blueprint.sync_version,
                self.counters,
            ),
            "encode": lambda: EncodeTextLane(
                RuntimeEngines(self.slots, priority=True), self.counters
            ),
            "collab": lambda: CollabLane(
                engines,
                ObjectStores(js, settings.collab.max_object_mib << 20),
                self.workspace,
                self.counters,
                settings.collab.max_object_mib,
                self.contract.lane("collab").result_object,
            ),
            "taste": lambda: TasteLane(
                engines,
                ObjectStores(js, settings.taste.max_object_mib << 20),
                self.workspace,
                self.counters,
                settings.taste,
            ),
        }
        for lane in self.blueprint.lanes:
            watch = watches[lane.name]
            watch.paused = True
            leases = Leases(
                lane,
                self.connection,
                self.counters,
                self.clock,
                watch_max_deliver(watch),
                self.contract.headers.msg_id,
            )
            if lane.rpc:
                refiner = build_refiner(
                    settings.llm, llm_session, engines, self.clock, self.counters
                )
                resolver = ArtistResolver(refiner, self.counters)
                matcher = TrackMatcher(refiner, self.counters)
                handler: RpcHandler | QueueHandler = RpcHandler(
                    lane,
                    {"resolve_artist": resolver.resolve, "match_track": matcher.match},
                    leases,
                    self.connection,
                    self.contract,
                    self.counters,
                    self.clock,
                    worker.id,
                    worker.build,
                )
            else:
                handler = QueueHandler(
                    lane,
                    processors[lane.name](),
                    leases,
                    Inflight(),
                    self.outbox,
                    watch,
                    self.contract,
                    self.producer(lane.name),
                    self.counters,
                    self.clock,
                )
            runner = LaneRunner(
                lane,
                watch,
                handler,
                js,
                self.connection,
                self.outbox,
                settings.lanes.capacity[lane.name],
                self.counters,
                self.clock,
            )
            self.lanes[lane.name] = ServedLane(lane, watch, runner)
        self.status = StatusReporter(
            self.connection,
            self.contract,
            settings,
            self.counters,
            self.clock,
            {name: served.runner for name, served in self.lanes.items()},
            self.outbox,
            self.blueprint.sync_version,
            self.supervisor.snapshot,
            self.gpu.snapshot,
        )

    def producer(self, lane: str) -> Producer:
        models = {
            role: settings_module.model_ref(self.settings, slot)
            for role, slot in PRODUCER_MODELS[lane].items()
        }
        sync_version = self.blueprint.sync_version if lane == "transcribe" else None
        return Producer(self.settings.worker.id, self.settings.worker.build, models, sync_version)

    def start_background(self) -> None:
        tasks: list[tuple[str, Awaitable[None]]] = [
            ("gate", self.gate()),
            ("status", self.status.run()),
            ("health", self.health.run(self.health_snapshot, self.health_stop)),
            ("gpu", self.gpu.run()),
        ]
        for name, served in self.lanes.items():
            tasks.append((f"watch:{name}", served.watch.run()))
            tasks.append((f"{LANE_TASK_PREFIX}{name}", served.runner.run()))
        for name, work in tasks:
            task = asyncio.ensure_future(work)
            task.set_name(name)
            task.add_done_callback(self.background_finished)
            self.background.append(task)

    def background_finished(self, task: asyncio.Task[None]) -> None:
        if task.cancelled():
            return
        error = task.exception()
        if error is not None:
            self.counters.inc("background_crashes_total", task=task.get_name())
            self.log.exception("background_task_crashed", error, task=task.get_name())
            self.crashed = True
            self.stop_requested.set()

    async def until_stopped(self) -> int:
        waits = [asyncio.ensure_future(self.stop_requested.wait())]
        waits += [
            asyncio.ensure_future(served.watch.drifted.wait()) for served in self.lanes.values()
        ]
        try:
            await asyncio.wait(waits, return_when=asyncio.FIRST_COMPLETED)
        finally:
            for wait in waits:
                wait.cancel()
            await asyncio.gather(*waits, return_exceptions=True)
        drifted = [name for name, served in self.lanes.items() if served.watch.drifted.is_set()]
        if drifted:
            self.log.error("consumer_drift_in_work", lanes=drifted)
            return EX_CONFIG
        return EXIT_CRASHED if self.crashed else 0

    async def serve_health(self) -> None:
        try:
            served = await self.status.serve_health()
        except nats.errors.Error as error:
            self.counters.inc("health_subscribe_failures_total")
            self.log.warning("health_subscribe_failed", error=str(error))
            return
        self.log.info("health_subject", served=served)

    async def shutdown(self, code: int) -> None:
        self.log.info("shutdown_started", exit_code=code)
        for served in self.lanes.values():
            served.runner.stop_fetching()
        await self.cancel_fetch_loops()
        await self.unless_hurried("drain", self.drain())
        await self.unless_hurried("outbox", self.flush_outbox())
        await self.unless_hurried("unsubscribe", self.unsubscribe())

    async def cancel_fetch_loops(self) -> None:
        loops = [task for task in self.background if task.get_name().startswith(LANE_TASK_PREFIX)]
        for task in loops:
            task.cancel()
        await asyncio.gather(*loops, return_exceptions=True)

    async def drain(self) -> None:
        grace = float(self.settings.runtime.shutdown_grace_s)
        left = await asyncio.gather(*(served.runner.drain(grace) for served in self.lanes.values()))
        for name, count in zip(self.lanes, left, strict=True):
            if count:
                self.log.error("lane_tasks_left_after_drain", lane=name, tasks=count)

    async def flush_outbox(self) -> None:
        if not await self.outbox.flush(OUTBOX_FLUSH_S):
            self.log.error("outbox_not_flushed", pending=self.outbox.pending)

    async def unsubscribe(self) -> None:
        for served in self.lanes.values():
            await served.runner.unsubscribe()
        await self.close_connection()

    async def unless_hurried(self, step: str, work: Awaitable[None]) -> None:
        if self.hurry.is_set():
            self.log.warning("shutdown_step_skipped", step=step)
            return
        task = asyncio.ensure_future(work)
        hurry = asyncio.ensure_future(self.hurry.wait())
        await asyncio.wait({task, hurry}, return_when=asyncio.FIRST_COMPLETED)
        hurry.cancel()
        if not task.done():
            self.log.warning("shutdown_step_cut", step=step)
            task.cancel()
        await asyncio.gather(hurry, return_exceptions=True)
        await asyncio.gather(task, return_exceptions=True)
        if not task.cancelled() and task.exception() is not None:
            error = task.exception()
            assert error is not None
            self.log.exception("shutdown_step_failed", error, step=step)

    async def stop_runtime(self) -> None:
        self.health_stop.set()
        for served in self.lanes.values():
            served.runner.stop_fetching()
        for task in self.background:
            if not task.done() and task.get_name() != "health":
                task.cancel()
        await asyncio.gather(*self.background, return_exceptions=True)
        for served in self.lanes.values():
            await served.runner.cancel()
        for batcher in self.batchers.values():
            await batcher.stop_intake()
        await self.supervisor.stop()
        for batcher in self.batchers.values():
            await batcher.close()

    async def close_connection(self) -> None:
        if self.connection.client.is_closed:
            return
        await self.connection.close()

    async def gate(self) -> None:
        while True:
            for name, served in self.lanes.items():
                self.gate_lane(name, served.watch)
            await self.clock.sleep(GATE_INTERVAL_S)

    def gate_lane(self, name: str, watch: ConsumerWatch) -> None:
        states = {slot: self.supervisor.slot_state(slot) for slot in LANE_GATES.get(name, ())}
        loading = [slot for slot, state in states.items() if state == STATE_LOADING]
        broken = [slot for slot, state in states.items() if state == STATE_BROKEN]
        if watch.paused == bool(loading) and watch.engine_broken == bool(broken):
            return
        watch.paused = bool(loading)
        watch.engine_broken = bool(broken)
        self.log.info(
            "lane_gate",
            lane=name,
            open=not (loading or broken),
            waiting_for=loading,
            broken=broken,
        )

    def health_snapshot(self) -> dict[str, object]:
        now = time.time()
        lanes: dict[str, object] = {}
        for name, served in self.lanes.items():
            state = served.runner.state.value
            previous = self.lane_states.get(name)
            since = previous[1] if previous is not None and previous[0] == state else now
            self.lane_states[name] = (state, since)
            lanes[name] = {STATE: state, SINCE: since}
        return {LANES: lanes}


@dataclass(frozen=True)
class HttpPools:
    llm: int
    audio: int

    @classmethod
    def of(cls, lanes: Iterable[LaneSpec], capacity: Mapping[str, int]) -> HttpPools:
        served = tuple(lanes)
        rpc = sum(capacity[lane.name] for lane in served if lane.rpc)
        fetching = sum(capacity[lane.name] for lane in served if lane.name in AUDIO_FETCH_LANES)
        return cls(
            llm=POOL_SPARE + LLM_CALLS_PER_REQUEST * rpc,
            audio=POOL_SPARE + fetching,
        )

    def llm_session(self) -> aiohttp.ClientSession:
        return aiohttp.ClientSession(connector=aiohttp.TCPConnector(limit=self.llm))

    def audio_session(self) -> aiohttp.ClientSession:
        return aiohttp.ClientSession(connector=aiohttp.TCPConnector(limit=self.audio))


@dataclass(frozen=True)
class ServedLane:
    spec: LaneSpec
    watch: ConsumerWatch
    runner: LaneRunner


class GpuProbe:
    def __init__(self, counters: Counters, *, enabled: bool) -> None:
        self.counters = counters
        self.enabled = enabled
        self.latest: dict[str, object] | None = None

    def snapshot(self) -> Mapping[str, object] | None:
        return self.latest

    async def run(self) -> None:
        while self.enabled:
            await self.probe()
            await asyncio.sleep(GPU_PROBE_INTERVAL_S)

    async def probe(self) -> None:
        try:
            process = await asyncio.create_subprocess_exec(
                *GPU_QUERY, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
            )
        except FileNotFoundError:
            log.warning("gpu_probe_unavailable", extra={"command": GPU_QUERY[0]})
            self.enabled = False
            return
        except OSError as error:
            self.failed(f"{type(error).__name__}: {error}")
            self.enabled = False
            return
        try:
            stdout, stderr = await asyncio.wait_for(process.communicate(), GPU_PROBE_TIMEOUT_S)
        except TimeoutError:
            process.kill()
            await process.wait()
            self.failed("timeout")
            return
        if process.returncode != 0:
            self.failed(stderr.decode(errors="replace").strip()[:200])
            return
        parsed = parse_gpu_line(stdout.decode(errors="replace"))
        if parsed is None:
            self.failed(f"unparsable output {stdout[:120]!r}")
            return
        self.latest = parsed

    def failed(self, detail: str) -> None:
        self.counters.inc("gpu_probe_failures_total")
        log.warning("gpu_probe_failed", extra={"detail": detail})


def parse_gpu_line(output: str) -> dict[str, object] | None:
    lines = [line for line in output.splitlines() if line.strip()]
    if not lines:
        return None
    parts = [part.strip() for part in lines[0].split(",")]
    if len(parts) != 3 or not parts[1].isdigit() or not parts[2].isdigit():
        return None
    return {"name": parts[0], "total_mib": int(parts[1]), "used_mib": int(parts[2])}


def watch_max_deliver(watch: ConsumerWatch) -> Callable[[], int]:
    return lambda: watch.max_deliver


def check_slow_lanes(settings: Settings) -> None:
    runtime = settings.runtime
    slow = sorted(SLOW_LANES & set(settings.lanes.enabled))
    if runtime.device == "cpu" and slow and not runtime.allow_slow_lanes:
        raise StartupError(f"lanes {slow} on device=cpu need runtime.allow_slow_lanes=true")


def slot_spec(settings: Settings, name: str) -> SlotSpec:
    loader = SLOT_LOADERS.get(name)
    if loader is None:
        raise StartupError(f"slot {name} has no engine class")
    slot = settings.slots[name]
    options: dict[str, object] = {}
    if name == settings_module.LOCAL_LLM_SLOT:
        options["quantize"] = settings.llm.local.quantize
    return SlotSpec(
        name=name,
        loader=loader,
        model=slot.model,
        revision=slot.revision,
        device="cpu" if name in CPU_ONLY_SLOTS else settings.runtime.device,
        max_batch=slot.max_batch,
        max_wait_ms=slot.max_wait_ms,
        options=options,
    )


def cpu_tool_specs(settings: Settings) -> tuple[SlotSpec, ...]:
    tools = settings.slots[CPU_TOOLS]
    return tuple(
        SlotSpec(
            name=name,
            loader=loader,
            model=tools.model if name == "lid" else "",
            revision=tools.revision if name == "lid" else "",
            device="cpu",
            max_batch=tools.max_batch,
            max_wait_ms=tools.max_wait_ms,
        )
        for name, loader in CPU_TOOL_LOADERS.items()
    )


def required_env(environ: Mapping[str, str], name: str) -> str:
    value = environ.get(name, "")
    if not value:
        raise StartupError(f"{name} is not set")
    return value
