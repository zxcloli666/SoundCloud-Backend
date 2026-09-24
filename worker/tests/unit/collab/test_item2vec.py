from __future__ import annotations

import json
import random
from pathlib import Path

import numpy as np
import pytest

from worker.models import item2vec
from worker.models.item2vec import Item2VecSlot, TrainParams, read_sessions, train
from worker.runtime.protocol import BadInput, SlotSpec

CLUSTERS = 400
CLUSTER_SIZE = 10
SPEC = SlotSpec("train-collab", "worker.models.item2vec:Item2VecSlot", "", "", "cpu", 1, 0)


def clustered_sessions(count: int, length: int, seed: int = 7) -> list[list[int]]:
    rng = random.Random(seed)
    sessions = []
    for _ in range(count):
        cluster = rng.randrange(CLUSTERS)
        start = rng.randrange(CLUSTER_SIZE)
        sessions.append(
            [
                1_000 + cluster * CLUSTER_SIZE + (start + step) % CLUSTER_SIZE
                for step in range(length)
            ]
        )
    return sessions


def write_input(path: Path, sessions: list[list[int]], version: int = 2) -> Path:
    path.write_text(json.dumps({"version": version, "sessions": sessions}, indent=1))
    return path


def params(tmp_path: Path, sessions_path: Path, min_count: int = 1) -> TrainParams:
    return TrainParams(
        sessions_path=sessions_path,
        vectors_path=tmp_path / "vectors.json",
        min_count=min_count,
        window=3,
        epochs=5,
        negative=5,
        workers=4,
    )


def test_learned_neighbours_beat_popularity(tmp_path: Path) -> None:
    source = write_input(tmp_path / "in.json", clustered_sessions(20_000, 8))

    training = train(params(tmp_path, source))

    assert training.sessions == 20_000
    assert training.vocab == CLUSTERS * CLUSTER_SIZE
    assert training.hr_at_20 > 0.8
    assert training.popularity_hr_at_20 < 0.05
    assert training.hr_at_20 > training.popularity_hr_at_20


def test_vectors_object_has_the_contract_shape(tmp_path: Path) -> None:
    source = write_input(tmp_path / "in.json", clustered_sessions(300, 6))

    training = train(params(tmp_path, source))
    written = json.loads((tmp_path / "vectors.json").read_text())

    assert written["dim"] == 128
    assert len(written["points"]) == training.vocab
    ids = {point["id"] for point in written["points"]}
    assert ids <= set(range(1_000, 1_000 + CLUSTERS * CLUSTER_SIZE))
    vector = np.asarray(written["points"][0]["vec"])
    assert vector.shape == (128,)
    assert np.linalg.norm(vector) == pytest.approx(1.0, abs=1e-5)
    assert written["metrics"] == {
        "hr_at_20": training.hr_at_20,
        "popularity_hr_at_20": training.popularity_hr_at_20,
        "sessions": 300,
        "vocab": training.vocab,
    }


def test_fewer_than_two_sessions_train_nothing(tmp_path: Path) -> None:
    source = write_input(tmp_path / "in.json", [[1, 2, 3]])

    training = train(params(tmp_path, source))

    assert (training.sessions, training.vocab) == (1, 0)
    assert not (tmp_path / "vectors.json").exists()


def test_min_count_above_every_count_leaves_an_empty_vocab(tmp_path: Path) -> None:
    source = write_input(tmp_path / "in.json", [[1, 2], [3, 4], [5, 6]])

    training = train(params(tmp_path, source, min_count=50))

    assert (training.sessions, training.vocab) == (3, 0)
    assert not (tmp_path / "vectors.json").exists()


def test_large_track_ids_survive_the_round_trip(tmp_path: Path) -> None:
    big = 2**64 - 1
    source = write_input(tmp_path / "in.json", [[big, 5, big, 5]] * 12)

    train(params(tmp_path, source))
    written = json.loads((tmp_path / "vectors.json").read_text())

    assert {point["id"] for point in written["points"]} == {big, 5}


@pytest.mark.parametrize("chunk", [1, 3, 17, 1 << 20])
def test_streaming_reader_is_chunk_independent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, chunk: int
) -> None:
    sessions = clustered_sessions(40, 5)
    source = tmp_path / "in.json"
    source.write_text(
        '{ "sessions" : [ ' + " , ".join(json.dumps(s) for s in sessions) + ' ] , "version": 2 }'
    )
    monkeypatch.setattr(item2vec, "CHUNK_CHARS", chunk)

    read = read_sessions(source)

    assert [read.session(index) for index in range(len(read))] == [
        [str(item) for item in session] for session in sessions
    ]


def test_empty_session_list_is_read(tmp_path: Path) -> None:
    source = write_input(tmp_path / "in.json", [])

    assert len(read_sessions(source)) == 0


@pytest.mark.parametrize(
    ("text", "problem"),
    [
        ('{"version": 1, "sessions": [[1]]}', "version"),
        ('{"version": 2}', "no 'sessions' array"),
        ('{"version": 2, "sessions": [[1, 2]', "not closed"),
        ('{"version": 2, "sessions": [[1, true]]}', "unsigned 64-bit"),
        ('{"version": 2, "sessions": [[1, -2]]}', "unsigned 64-bit"),
        ('{"version": 2, "sessions": [[1, "2"]]}', "unsigned 64-bit"),
        ('{"version": 2, "sessions": [[1, 18446744073709551616]]}', "unsigned 64-bit"),
        ('{"version": 2, "sessions": [{"a": 1}]}', "array of track ids"),
        ('{"version": 2, "sessions": [[1, 2]] garbage', "not valid JSON"),
        ('{"version": 2, "sessions": [[1, 2 3]]}', "not valid JSON"),
    ],
)
def test_malformed_input_is_bad_input(tmp_path: Path, text: str, problem: str) -> None:
    source = tmp_path / "in.json"
    source.write_text(text)

    with pytest.raises(BadInput, match=problem):
        read_sessions(source)


def test_non_utf8_input_is_bad_input(tmp_path: Path) -> None:
    source = tmp_path / "in.json"
    source.write_bytes(b'{"version": 2, "sessions": [[1]], "x": "\xff"}')

    with pytest.raises(BadInput, match="not valid JSON"):
        read_sessions(source)


def test_slot_trains_through_invoke(tmp_path: Path) -> None:
    sessions = clustered_sessions(200, 6)
    source = write_input(tmp_path / "in.json", sessions)
    slot = Item2VecSlot()
    slot.load(SPEC)
    slot.warmup()

    arrays, result = slot.invoke(
        "train",
        {},
        {
            "sessions_path": str(source),
            "vectors_path": str(tmp_path / "vectors.json"),
            "min_count": 1,
            "window": 3,
            "epochs": 3,
            "negative": 5,
        },
    )

    assert arrays == {}
    assert result["sessions"] == 200
    distinct = {item for index, session in enumerate(sessions) if index % 10 for item in session}
    assert result["vocab"] == result["points_count"] == len(distinct)
    assert isinstance(result["hr_at_20"], float)


def test_slot_rejects_bad_arguments_and_unknown_methods(tmp_path: Path) -> None:
    slot = Item2VecSlot()
    with pytest.raises(RuntimeError, match="not loaded"):
        slot.invoke("train", {}, {})
    slot.load(SPEC)

    with pytest.raises(BadInput, match="min_count"):
        slot.invoke("train", {}, {"sessions_path": "a", "vectors_path": "b", "min_count": 0})
    with pytest.raises(ValueError, match="no method"):
        slot.invoke("embed", {}, {})
    slot.unload()
    with pytest.raises(RuntimeError, match="not loaded"):
        slot.invoke("train", {}, {})
