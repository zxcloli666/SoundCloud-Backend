from __future__ import annotations

import nats.js.errors
from nats.js import JetStreamContext, api
from nats.js.manager import JetStreamManager

from worker.contract import Contract, LaneSpec, StreamSpec

RETENTION = {
    "work_queue": api.RetentionPolicy.WORK_QUEUE,
    "limits": api.RetentionPolicy.LIMITS,
    "interest": api.RetentionPolicy.INTEREST,
}
DISCARD = {"new": api.DiscardPolicy.NEW, "old": api.DiscardPolicy.OLD}


async def provision_like_jobs(
    jsm: JetStreamManager, js: JetStreamContext, contract: Contract
) -> None:
    for stream in contract.streams.values():
        await jsm.add_stream(stream_config(stream))
    for lane in contract.lanes.values():
        await jsm.add_consumer(lane.stream, consumer_config(lane))
    for bucket, spec in contract.object_stores.items():
        await js.create_object_store(
            bucket, config=api.ObjectStoreConfig(bucket=bucket, ttl=spec.max_age_s)
        )


async def reset(jsm: JetStreamManager, contract: Contract) -> None:
    names = [*contract.streams, *(f"OBJ_{bucket}" for bucket in contract.object_stores)]
    for name in names:
        try:
            await jsm.delete_stream(name)
        except nats.js.errors.NotFoundError:
            continue


def stream_config(stream: StreamSpec) -> api.StreamConfig:
    return api.StreamConfig(
        name=stream.name,
        subjects=list(stream.subjects),
        retention=RETENTION[stream.retention],
        max_age=float(stream.max_age_s),
        duplicate_window=float(stream.duplicate_window_s),
        max_bytes=stream.max_bytes,
        discard=DISCARD[stream.discard],
        storage=api.StorageType.FILE,
    )


def consumer_config(lane: LaneSpec, **overrides: object) -> api.ConsumerConfig:
    config = api.ConsumerConfig(
        durable_name=lane.durable,
        name=lane.durable,
        filter_subject=lane.filter_subject,
        ack_policy=api.AckPolicy.EXPLICIT,
        deliver_policy=api.DeliverPolicy.ALL,
        replay_policy=api.ReplayPolicy.INSTANT,
        ack_wait=lane.ack_wait_s,
        max_deliver=lane.max_deliver,
        max_ack_pending=lane.max_ack_pending,
    )
    for key, value in overrides.items():
        setattr(config, key, value)
    return config
