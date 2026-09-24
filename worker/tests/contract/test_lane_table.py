from __future__ import annotations

import pytest

from worker.contract import Contract

MAX_DELIVER = 5
NONE = "-"
LANE_TABLE = """
audio       INDEX_AUDIO   index.audio.new       180  60  12 256  30  600  1650  240 public
lyrics      EMBED_LYRICS  embed.lyrics.new       60  30   6 512  15  300   825  120 public
transcribe  TRANSCRIBE    transcribe.audio.new  900 120  24  64  60  900  5700  960 public
encode      ENCODE        encode.text.new        30  30   6 256   5   60   225    - private
collab      TRAIN_COLLAB  train.collab.new     3600 300  60   1 120 1800 19800    - private
taste       TRAIN_TASTE   train.taste.new      7200 300  60   1 300 1800 39900    - private
ai          AI_RPC        ai.rpc.>               20  30   6 128   -    -     -    - private
"""
STREAM_TABLE = """
INDEX_AUDIO     index.audio.>     work_queue   86400  86400
EMBED_LYRICS    embed.lyrics.>    work_queue   86400  86400
TRANSCRIBE      transcribe.>      work_queue   86400  86400
ENCODE          encode.>          work_queue   86400    900
TRAIN_COLLAB    train.collab.>    work_queue   21600   3600
TRAIN_TASTE     train.taste.>     work_queue   86400   3600
AI_RPC          ai.rpc.>          work_queue     120    120
PIPELINE_DONE   done.>            limits      259200  43200
WORKER_INVALID  worker.invalid.>  limits      604800    120
"""
DONE_SUBJECTS = {
    "audio": "done.index_audio",
    "lyrics": "done.embed_lyrics",
    "transcribe": "done.transcribe",
    "encode": "done.encode",
    "collab": "done.train_collab",
    "taste": "done.train_taste",
    "ai": None,
}
OBJECT_STORES = {"COLLAB_DATA": 86400, "TASTE_DATA": 86400, "TASTE_MODELS": None}


def rows(table: str) -> dict[str, list[str]]:
    return {row.split()[0]: row.split()[1:] for row in table.strip().splitlines()}


def number(cell: str) -> float | None:
    return None if cell == NONE else float(cell)


LANES = rows(LANE_TABLE)
STREAMS = rows(STREAM_TABLE)


@pytest.mark.parametrize("name", sorted(LANES))
def test_lane_matches_the_design_table(contract: Contract, name: str) -> None:
    stream, filter_subject, *numbers, visibility = LANES[name]
    lane = contract.lane(name)
    assert (lane.stream, lane.filter_subject) == (stream, filter_subject)
    assert lane.durable == f"{name}-workers"
    actual = (
        lane.deadline_s,
        lane.ack_wait_s,
        lane.heartbeat_s,
        lane.max_ack_pending,
        lane.nak_base_s,
        lane.nak_cap_s,
        lane.attempt_window_s,
        lane.bridge_ttl_s,
    )
    assert actual == tuple(number(cell) for cell in numbers)
    assert lane.public == (visibility == "public")
    assert lane.max_deliver == MAX_DELIVER
    assert lane.done_subject == DONE_SUBJECTS[name]


@pytest.mark.parametrize("name", sorted(STREAMS))
def test_stream_matches_the_design_table(contract: Contract, name: str) -> None:
    subject, retention, max_age_s, duplicate_window_s = STREAMS[name]
    stream = contract.streams[name]
    assert stream.subjects == (subject,)
    assert stream.retention == retention
    assert (stream.max_age_s, stream.duplicate_window_s) == (
        int(max_age_s),
        int(duplicate_window_s),
    )


def test_every_contract_lane_stream_and_bucket_is_in_the_design(contract: Contract) -> None:
    assert set(contract.lanes) == set(LANES)
    assert set(contract.streams) == set(STREAMS)
    stores = {name: store.max_age_s for name, store in contract.object_stores.items()}
    assert stores == OBJECT_STORES


def test_lane_filters_live_in_their_streams(contract: Contract) -> None:
    for lane in contract.lanes.values():
        prefix = contract.streams[lane.stream].subjects[0].removesuffix(">")
        assert lane.filter_subject.startswith(prefix)
