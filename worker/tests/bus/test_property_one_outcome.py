from __future__ import annotations

import asyncio
from collections import Counter
from collections.abc import Mapping
from dataclasses import dataclass, field

from hypothesis import given, settings
from hypothesis import strategies as st

from tests.bus.conftest import Harness, build_harness
from tests.conftest import BASE_ENV, CONFIG_DIR, CONTRACT_PATH
from tests.fakes.clock import FakeClock
from tests.fakes.jetstream import FakeNats
from worker import contract as contract_module
from worker import settings as settings_module
from worker.bus.lease import drop_if_stale
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason, TransientFailure

CONTRACT = contract_module.load(CONTRACT_PATH)
SETTINGS = settings_module.load(CONFIG_DIR, BASE_ENV)
LANE = CONTRACT.lane("audio")
WINDOW_S = LANE.ack_wait_s - LANE.heartbeat_s
TERMINAL = {"ack_sync", "nak", "term"}

behaviour = st.sampled_from(["ok", "transient", "permanent", "hold", "drop"])
task = st.fixed_dictionaries(
    {"track": st.sampled_from(["1", "2"]), "attempt": st.sampled_from([1, 2])}
)
event = st.one_of(
    st.tuples(st.just("tick"), st.sampled_from([1.0, 5.0, 13.0, 49.0, 61.0, 700.0])),
    st.tuples(st.just("disconnect"), st.just(0.0)),
    st.tuples(st.just("reconnect"), st.just(0.0)),
    st.tuples(st.just("lose_puback"), st.just(0.0)),
    st.tuples(st.just("resume"), st.just(0.0)),
    st.tuples(st.just("enqueue"), st.just(0.0)),
)
scenario = st.fixed_dictionaries(
    {
        "tasks": st.lists(task, min_size=1, max_size=3),
        "behaviours": st.lists(behaviour, min_size=1, max_size=8),
        "events": st.lists(event, min_size=1, max_size=12),
    }
)


@dataclass
class Journal:
    deliveries: dict[tuple[int, int], float] = field(default_factory=dict)
    sent: list[tuple[str, int, int, float]] = field(default_factory=list)
    holds: int = 0


def payload_of(spec: Mapping[str, object]) -> dict[str, object]:
    return {
        "sc_track_id": spec["track"],
        "s3_url": "https://s3/x",
        "upload_generation": 1,
        "attempt": spec["attempt"],
    }


def scripted(harness: Harness, behaviours: list[str], journal: Journal) -> None:
    calls = Counter[str]()

    async def process(request: Mapping[str, object], deadline: Deadline) -> Outcome:
        key = LANE.correlation_key(request)
        index = calls[key]
        calls[key] += 1
        kind = behaviours[index % len(behaviours)]
        if kind == "hold":
            journal.holds += 1
            await harness.processor.release.wait()
            drop_if_stale()
        if kind == "transient":
            raise TransientFailure(Reason.DOWNLOAD_FAILED, "scripted")
        if kind == "permanent":
            raise PermanentFailure(Reason.UNDECODABLE_AUDIO, "scripted")
        if kind == "drop":
            drop_if_stale()
        return Outcome.ok(mert=[0.0] * 1024, clap=[0.0] * 512, fingerprint=None)

    harness.handler._processor = type("P", (), {"process": staticmethod(process)})()


def observe(harness: Harness, journal: Journal) -> None:
    bus = harness.bus
    attach = harness.leases.attach
    send_ack = bus.send_ack
    send_ack_sync = bus.send_ack_sync

    def recording_attach(msg, payload, correlation):
        attached = attach(msg, payload, correlation)
        journal.deliveries[(attached.lease.stream_seq, attached.lease.num_delivered)] = (
            harness.clock.now()
        )
        return attached

    async def recording_send_ack(message, kind, delay):
        journal.sent.append((kind, message.seq, message.metadata.num_delivered, bus.clock.now()))
        await send_ack(message, kind, delay)

    async def recording_send_ack_sync(message, timeout):
        await send_ack_sync(message, timeout)
        journal.sent.append(
            ("ack_sync", message.seq, message.metadata.num_delivered, bus.clock.now())
        )

    harness.leases.attach = recording_attach
    bus.send_ack = recording_send_ack
    bus.send_ack_sync = recording_send_ack_sync


async def run_scenario(plan: Mapping[str, object]) -> None:
    clock = FakeClock()
    bus = FakeNats(clock)
    from tests.conftest import provision_like_jobs

    provision_like_jobs(bus, CONTRACT)
    harness = await build_harness(bus, CONTRACT, SETTINGS, clock, "audio", capacity=2)
    journal = Journal()
    scripted(harness, list(plan["behaviours"]), journal)
    observe(harness, journal)
    harness.processor.hold()
    await harness.watch.check()
    harness.start()
    tasks = list(plan["tasks"])
    pending = list(tasks)
    seqs: dict[int, str] = {}

    def enqueue() -> None:
        if not pending:
            return
        spec = pending.pop(0)
        payload = payload_of(spec)
        seq = bus.enqueue(LANE.filter_subject, payload, {"Nats-Msg-Id": f"t:{len(seqs)}"})
        seqs[seq] = LANE.correlation_key(payload)

    enqueue()
    for kind, amount in plan["events"]:
        if kind == "tick":
            await clock.tick(amount, turns=20)
        elif kind == "disconnect" and bus.is_connected:
            await bus.disconnect()
        elif kind == "reconnect" and not bus.is_connected:
            await bus.reconnect()
        elif kind == "lose_puback":
            bus.lost_pubacks += 1
        elif kind == "resume":
            harness.processor.resume()
        elif kind == "enqueue":
            enqueue()
        await harness.settle(5)
    while pending:
        enqueue()
    if not bus.is_connected:
        await bus.reconnect()
    harness.processor.resume()
    bus.lost_pubacks = 0
    consumer = bus.consumers[("INDEX_AUDIO", "audio-workers")]
    for _ in range(600):
        await clock.tick(10.0, turns=20)
        open_seqs = set(bus.streams["INDEX_AUDIO"].messages) - consumer.exhausted
        if not open_seqs and not harness.runner.tasks:
            break
    await harness.settle(50)
    try:
        check_invariants(harness, journal, seqs)
    finally:
        await harness.stop()


def check_invariants(harness: Harness, journal: Journal, seqs: dict[int, str]) -> None:
    terminal = Counter((seq, n) for kind, seq, n, _ in journal.sent if kind in TERMINAL)
    assert all(count == 1 for count in terminal.values()), terminal
    published = {
        headers["Nats-Msg-Id"]
        for subject, _, headers in harness.bus.published
        if subject.startswith("done.")
    }
    for kind, seq, _, _ in journal.sent:
        if kind == "ack_sync":
            correlation = seqs[seq]
            assert any(f":{correlation}:" in msg_id for msg_id in published), (seq, published)
    for kind, seq, n, at in journal.sent:
        if kind not in ("nak", "in_progress"):
            continue
        since = journal.deliveries[(seq, n)]
        for other, other_seq, other_n, other_at in journal.sent:
            if other == "in_progress" and (other_seq, other_n) == (seq, n) and other_at < at:
                since = max(since, other_at)
        assert at - since <= WINDOW_S + 1e-9, (kind, seq, n, at - since)
    stream = harness.bus.streams["INDEX_AUDIO"]
    consumer = harness.bus.consumers[("INDEX_AUDIO", "audio-workers")]
    for seq in seqs:
        assert seq not in stream.messages or seq in consumer.exhausted, seq
    assert harness.counters.value("lease_unsettled_total", lane="audio") == 0
    assert harness.counters.value("handler_crashes_total", lane="audio") == 0


@settings(max_examples=60, deadline=None)
@given(scenario)
def test_every_delivery_ends_in_exactly_one_outcome(plan: Mapping[str, object]) -> None:
    asyncio.run(run_scenario(plan))
