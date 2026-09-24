from __future__ import annotations

import base64
import json
import re
from dataclasses import replace
from pathlib import Path

import numpy as np
import orjson
import pytest
import torch
from safetensors.torch import load_file

from tests.unit.taste.synthetic import EVENT_CODES, SINCE, UNTIL, Mode, World, write_input
from worker.domain.outcome import Reason, Status
from worker.domain.taste import TasteTraining, judge
from worker.engines import taste_training
from worker.models import taste_trainer
from worker.models.taste_trainer import (
    BASELINES,
    SHORT_HISTORY,
    Evaluation,
    Scores,
    Split,
    TasteTrainerSlot,
    Training,
    TrainParams,
    everything,
    held_out,
    positive_times,
    profile_of,
    read_dataset,
    recent_negatives,
    train,
)
from worker.runtime.protocol import BadInput, SlotSpec

CPU = torch.device("cpu")
MIN_USERS = 500
TRAINED_AT = 1_790_000_000
UNIT_WORLD = World(users=560, tracks=1600)
SPEC = SlotSpec("train-taste", "worker.models.taste_trainer:TasteTrainerSlot", "", "", "cpu", 1, 0)
LIKE, IMPORT, PLAYLIST, PLAY, SKIP, DISLIKE = (EVENT_CODES[name] for name in EVENT_CODES)


def params(folder: Path, source: Path, epochs: int = 8, budget_s: int = 3600) -> TrainParams:
    return TrainParams(
        input_path=source,
        artifact_path=folder / "artifact.json",
        tower_path=folder / "tower.safetensors",
        epochs=epochs,
        batch_size=256,
        negatives=256,
        seed=1,
        min_users=MIN_USERS,
        budget_s=budget_s,
        trained_at=TRAINED_AT,
    )


def as_domain(training: Training) -> TasteTraining:
    return taste_training(orjson.loads(orjson.dumps(training.to_result())))


@pytest.fixture(scope="module")
def taste_run(tmp_path_factory: pytest.TempPathFactory) -> tuple[Path, Training]:
    folder = tmp_path_factory.mktemp("taste")
    source = write_input(folder / "input.jsonl", UNIT_WORLD)
    return folder, train(params(folder, source), CPU)


def test_the_model_beats_every_baseline_on_data_with_a_planted_taste(
    taste_run: tuple[Path, Training],
) -> None:
    _, training = taste_run
    domain = as_domain(training)
    assert domain.model is not None and domain.baselines is not None

    best_recall = max(scores.recall_at_50 for scores in domain.baselines.values())
    best_ndcg = max(scores.ndcg_at_20 for scores in domain.baselines.values())
    assert training.test_users >= MIN_USERS
    assert domain.model.recall_at_50 >= 1.5 * best_recall
    assert domain.model.ndcg_at_20 > best_ndcg
    assert judge(domain, MIN_USERS) is None


def test_the_artifact_holds_every_featured_track_and_the_pooling(
    taste_run: tuple[Path, Training],
) -> None:
    folder, training = taste_run
    artifact = json.loads((folder / "artifact.json").read_text())

    assert training.version is not None
    assert re.fullmatch(r"taste-202609211413-[0-9a-f]{8}", training.version)
    assert training.tower_object == training.version + "-tower"
    assert artifact["version"] == training.version
    assert artifact["trained_at"] == "2026-09-21T14:13:20Z"
    assert artifact["dim"] == 128
    assert artifact["tower"]["object"] == training.tower_object
    assert set(artifact["pooling"]["w"]) == set(EVENT_CODES)
    assert artifact["pooling"]["w"]["skip"] < 0 < artifact["pooling"]["w"]["like"]
    assert artifact["pooling"]["tau_days"] > 0
    assert artifact["pooling"]["max_events"] == 200
    assert artifact["pooling"]["min_positives"] in (1, SHORT_HISTORY)
    assert artifact["metrics"]["evaluated_users"] == training.evaluated_users >= MIN_USERS
    assert artifact["metrics"]["fine_tune"]["steps"] > 0
    assert SINCE < artifact["metrics"]["horizon"] < UNTIL
    assert artifact["metrics"]["epochs_done"] == 8
    assert set(artifact["metrics"]["baselines"]) == set(BASELINES)
    assert artifact["metrics"]["segments"]["short_history"]["model"]["users"] > 0
    items = artifact["items"]
    assert len(items) == UNIT_WORLD.tracks == training.items_count
    vectors = np.array([item["vec"] for item in items], dtype=np.float32)
    assert vectors.shape == (UNIT_WORLD.tracks, 128)
    assert np.allclose(np.linalg.norm(vectors, axis=1), 1.0, atol=1e-4)


def test_the_tower_file_is_the_content_mlp_without_ids(taste_run: tuple[Path, Training]) -> None:
    folder, _ = taste_run

    tower = load_file(str(folder / "tower.safetensors"))

    assert {name: tuple(tensor.shape) for name, tensor in tower.items()} == {
        "hidden.weight": (512, 1664),
        "hidden.bias": (512,),
        "out.weight": (128, 512),
        "out.bias": (128,),
    }


def test_the_serving_version_is_scored_on_listeners_it_never_saw(
    taste_run: tuple[Path, Training], tmp_path: Path
) -> None:
    folder, _ = taste_run
    artifact = json.loads((folder / "artifact.json").read_text())
    artifact["data"]["until"] = SINCE
    previous = tmp_path / "previous.json"
    previous.write_text(json.dumps(artifact))

    training = train(
        replace(params(tmp_path, folder / "input.jsonl", epochs=1), previous_path=previous), CPU
    )
    domain = as_domain(training)

    assert training.previous_state == "compared"
    assert domain.previous is not None
    assert domain.previous.users == training.evaluated_users
    assert domain.previous.previous.recall_at_50 > 0.0


@pytest.mark.parametrize(
    ("until", "body", "state"),
    [
        (UNTIL, None, "too_few_fresh"),
        (SINCE, b'{"items": "nope"}', "unreadable"),
        (SINCE, b"not json", "unreadable"),
    ],
)
def test_a_serving_version_that_cannot_be_compared_says_why(
    taste_run: tuple[Path, Training], tmp_path: Path, until: int, body: bytes | None, state: str
) -> None:
    folder, _ = taste_run
    artifact = json.loads((folder / "artifact.json").read_text())
    artifact["data"]["until"] = until
    previous = tmp_path / "previous.json"
    previous.write_bytes(body if body is not None else json.dumps(artifact).encode())
    source = write_input(tmp_path / "input.jsonl", replace_world(users=40))

    training = train(
        replace(params(tmp_path, source, epochs=1), min_users=5, previous_path=previous), CPU
    )

    assert training.previous_state == state
    assert as_domain(training).previous is None


def test_popularity_alone_is_not_beaten_by_the_margin(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", replace_world(mode=Mode.POPULARITY))

    training = train(params(tmp_path, source), CPU)
    verdict = judge(as_domain(training), MIN_USERS)

    assert verdict is not None
    assert (verdict.status, verdict.reason) == (Status.REJECTED, Reason.BELOW_BASELINE)


def test_the_same_input_and_seed_give_the_same_metrics_on_cpu(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", UNIT_WORLD)
    first = tmp_path / "first"
    second = tmp_path / "second"
    first.mkdir()
    second.mkdir()

    one = train(params(first, source, epochs=2), CPU)
    two = train(params(second, source, epochs=2), CPU)

    assert one.to_result() == two.to_result()
    assert (first / "artifact.json").read_bytes() == (second / "artifact.json").read_bytes()


def test_fewer_test_users_than_the_gate_skip_training(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", replace_world(users=120))

    training = train(params(tmp_path, source), CPU)
    verdict = judge(as_domain(training), MIN_USERS)

    assert training.to_result()["trained"] is False
    assert training.test_users < MIN_USERS
    assert not (tmp_path / "artifact.json").exists()
    assert verdict is not None and verdict.reason is Reason.TOO_FEW_USERS


def test_imported_likes_alone_have_no_test_slice(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", replace_world(mode=Mode.IMPORTS_ONLY))

    training = train(params(tmp_path, source), CPU)

    assert training.test_users == 0
    assert training.users_count == UNIT_WORLD.users
    assert judge(as_domain(training), MIN_USERS) is not None


def test_a_budget_spent_before_the_first_step_publishes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = write_input(tmp_path / "input.jsonl", UNIT_WORLD)
    ticks = iter(range(0, 10**9, 10_000))
    monkeypatch.setattr(taste_trainer.time, "monotonic", lambda: float(next(ticks)))

    training = train(params(tmp_path, source, budget_s=1), CPU)
    verdict = judge(as_domain(training), MIN_USERS)

    assert (training.epochs_done, training.steps, training.budget_spent) == (0, 0, True)
    assert training.to_result()["trained"] is False
    assert not (tmp_path / "artifact.json").exists()
    assert verdict is not None
    assert (verdict.status, verdict.reason) == (Status.FAILED, Reason.DEADLINE_EXCEEDED)


def test_reading_the_input_does_not_eat_the_training_budget(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = write_input(tmp_path / "input.jsonl", replace_world(users=40))
    clock = [0.0]
    slow_read = taste_trainer.read_dataset

    def read_for_an_hour(path: Path) -> taste_trainer.Dataset:
        clock[0] += 3600.0
        return slow_read(path)

    monkeypatch.setattr(taste_trainer.time, "monotonic", lambda: clock[0])
    monkeypatch.setattr(taste_trainer, "read_dataset", read_for_an_hour)

    training = train(replace(params(tmp_path, source, epochs=1, budget_s=60), min_users=5), CPU)

    assert training.steps > 0
    assert training.budget_spent is False


def test_the_slot_trains_through_invoke(tmp_path: Path) -> None:
    source = write_input(tmp_path / "input.jsonl", replace_world(users=40))
    slot = TasteTrainerSlot()
    slot.load(SPEC)
    slot.warmup()

    _, result = slot.invoke("train", {}, slot_args(tmp_path, source))

    assert result["trained"] is False
    assert result["users_count"] == 40
    with pytest.raises(BadInput):
        slot.invoke("fit", {}, {})
    with pytest.raises(BadInput):
        slot.invoke("train", {}, {**slot_args(tmp_path, source), "seed": 0})


def slot_args(folder: Path, source: Path) -> dict[str, object]:
    return {
        "input_path": str(source),
        "artifact_path": str(folder / "artifact.json"),
        "tower_path": str(folder / "tower.safetensors"),
        "epochs": 2,
        "batch_size": 256,
        "negatives": 256,
        "seed": 1,
        "min_users": MIN_USERS,
        "budget_s": 3600,
        "trained_at": TRAINED_AT,
    }


def replace_world(**changes: object) -> World:
    return World(**{**vars(UNIT_WORLD), **changes})


def test_repeated_plays_become_a_positive_at_the_second_play() -> None:
    tracks = np.array([7, 7, 8, 9, 9], dtype=np.uint64)
    kinds = np.array([PLAY, PLAY, PLAY, LIKE, DISLIKE], dtype=np.int8)
    times = np.array([100.0, 300.0, 50.0, 10.0, 20.0])

    assert positive_times(tracks, kinds, times) == {7: 300.0}


def test_imported_likes_have_no_time_and_playlists_keep_the_first_moment() -> None:
    tracks = np.array([1, 2, 2, 3], dtype=np.uint64)
    kinds = np.array([IMPORT, PLAYLIST, LIKE, IMPORT], dtype=np.int8)
    times = np.array([np.nan, 500.0, 400.0, np.nan])

    positives = positive_times(tracks, kinds, times)

    assert np.isnan(positives[1])
    assert positives[2] == 400.0
    assert np.isnan(positives[3])


def test_an_imported_like_keeps_the_time_of_the_same_pair_in_a_playlist() -> None:
    tracks = np.array([1, 1, 2, 2, 2], dtype=np.uint64)
    kinds = np.array([IMPORT, PLAYLIST, IMPORT, PLAY, PLAY], dtype=np.int8)
    times = np.array([np.nan, 30.0, np.nan, 40.0, 50.0])

    assert positive_times(tracks, kinds, times) == {1: 30.0, 2: 50.0}


def test_the_test_slice_is_the_last_tenth_of_timed_positives(tmp_path: Path) -> None:
    events: list[list[object]] = [[1000 + n, LIKE, SINCE + n * 86_400, 1.0] for n in range(12)]
    events.append([2000, IMPORT, None, 1.0])
    events.append([1011, PLAY, SINCE + 12 * 86_400, 0.3])
    dataset = read_dataset(jsonl(tmp_path, [events], range(1000, 1012)))

    held, timed = held_out(dataset.users[0], dataset)

    assert held is not None
    profile = held.profile
    assert timed == 12
    assert sorted(dataset.track_ids[profile.test].tolist()) == [1010, 1011]
    assert sorted(dataset.track_ids[profile.positives].tolist()) == list(range(1000, 1010))
    assert profile.items.size == 10
    assert profile.ages[-1] == pytest.approx(1.0)
    assert 1010 not in dataset.track_ids[profile.items].tolist()


def test_no_event_of_a_test_track_reaches_the_history_it_is_predicted_from(
    tmp_path: Path,
) -> None:
    day = 86_400
    events: list[list[object]] = [[1000 + n, LIKE, SINCE + n * day, 1.0] for n in range(9)]
    events += [
        [1009, PLAY, SINCE + 3 * day, 0.3],
        [1009, PLAY, SINCE + 9 * day, 0.3],
        [1010, PLAY, SINCE + 4 * day, 0.3],
        [1010, LIKE, SINCE + 10 * day, 1.0],
        [1011, SKIP, SINCE + 5 * day, -0.3],
        [1012, PLAY, SINCE + 6 * day, 0.3],
    ]
    dataset = read_dataset(jsonl(tmp_path, [events], range(1000, 1013)))

    held, _ = held_out(dataset.users[0], dataset)

    assert held is not None
    history = set(dataset.track_ids[held.profile.items].tolist())
    assert sorted(dataset.track_ids[held.profile.test].tolist()) == [1009, 1010]
    assert history.isdisjoint({1009, 1010})
    assert {1011, 1012} <= set(dataset.track_ids[held.profile.seen].tolist())


def test_a_user_below_five_timed_positives_is_not_held_out(tmp_path: Path) -> None:
    events: list[list[object]] = [[1000 + n, LIKE, SINCE + n, 1.0] for n in range(4)]
    events.append([1004, IMPORT, None, 1.0])
    dataset = read_dataset(jsonl(tmp_path, [events], range(1000, 1005)))
    user = dataset.users[0]

    held, timed = held_out(user, dataset)
    profile = profile_of(user, everything(user), float(UNTIL), (), dataset)

    assert (held, timed) == (None, 4)
    assert profile.test.size == 0
    assert profile.positives.size == 5


def test_nothing_after_the_evaluation_horizon_is_trained_on(tmp_path: Path) -> None:
    day = 86_400
    users: list[list[list[object]]] = []
    for user in range(4):
        start = SINCE + user * 20 * day
        users.append([[1000 + n, LIKE, start + n * day, 1.0] for n in range(10)])
    users.append([[1000, LIKE, SINCE + 170 * day, 1.0], [1001, PLAY, SINCE + 171 * day, 0.3]])
    dataset = read_dataset(jsonl(tmp_path, users, range(1000, 1010)))

    split = Split.of(dataset, min_users=2)

    assert split.horizon == SINCE + 49 * day
    assert [profile.reference for profile in split.test_profiles] == [
        SINCE + 49 * day,
        SINCE + 69 * day,
    ]
    assert all(np.all(profile.ages > 0) for profile in split.past.profiles)
    assert [profile.items.size for profile in split.past.profiles] == [10, 10, 9, 0, 0]
    assert split.past.positive_users[0] == 3
    assert split.full.positive_users[0] == 5
    assert split.past.event_counts[1] == 3
    assert split.full.event_counts[1] == 5


def test_the_freshest_explicit_negatives_are_kept() -> None:
    items = np.arange(40, dtype=np.int64)
    kinds = np.full(40, DISLIKE, dtype=np.int8)
    times = np.arange(40, 0, -1, dtype=np.float64)
    weights = np.full(40, -1.0, dtype=np.float32)

    negatives = recent_negatives(items, kinds, times, weights)

    assert sorted(negatives.tolist()) == list(range(16))


def test_a_mild_skip_is_no_explicit_negative() -> None:
    items = np.array([1, 2, 3], dtype=np.int64)
    kinds = np.array([SKIP, SKIP, PLAY], dtype=np.int8)
    times = np.array([10.0, 20.0, 30.0])
    weights = np.array([0.0, -0.3, 0.3], dtype=np.float32)

    assert recent_negatives(items, kinds, times, weights).tolist() == [2]


def test_short_histories_are_only_served_when_the_model_wins_them() -> None:
    def evaluation(model_ndcg: float, users: int) -> Evaluation:
        scores = {name: Scores(0.1, 0.1, 0.0, 0.1, users, 0) for name in BASELINES}
        model = Scores(0.3, model_ndcg, 0.0, 0.1, users, 0)
        overall = {"model": model, **scores}
        return Evaluation(overall=overall, short_history={"model": model, **scores})

    assert evaluation(0.2, 40).min_positives() == 1
    assert evaluation(0.05, 40).min_positives() == SHORT_HISTORY
    assert evaluation(0.05, 0).min_positives() == 1


@pytest.mark.parametrize(
    ("line", "message"),
    [
        (b'{"version":2,"since":0,"until":1,"event_types":{}}', "version"),
        (b'{"version":1,"since":0,"until":1,"event_types":{"like":0}}', "event_types"),
        (b"not json", "JSON"),
    ],
)
def test_malformed_headers_are_bad_input(tmp_path: Path, line: bytes, message: str) -> None:
    path = tmp_path / "input.jsonl"
    path.write_bytes(line + b"\n")

    with pytest.raises(BadInput, match=message):
        read_dataset(path)


@pytest.mark.parametrize(
    "body",
    [
        {"u": "a", "e": [[1, 9, 10, 1.0]]},
        {"u": "a", "e": [[-1, 0, 10, 1.0]]},
        {"u": "a", "e": [[1, 0, "10", 1.0]]},
        {"u": "a", "e": [[1, 0, 10]]},
        {"i": 1, "clap": "!!", "mert": "", "collab": None},
        {"i": 1, "clap": base64.b64encode(b"\0" * 10).decode(), "mert": "", "collab": None},
        {"x": 1},
    ],
)
def test_malformed_lines_are_bad_input(tmp_path: Path, body: dict[str, object]) -> None:
    path = tmp_path / "input.jsonl"
    header = {"version": 1, "since": SINCE, "until": UNTIL, "event_types": EVENT_CODES}
    path.write_bytes(orjson.dumps(header) + b"\n" + orjson.dumps(body) + b"\n")

    with pytest.raises(BadInput):
        read_dataset(path)


def jsonl(folder: Path, users: list[list[list[object]]], featured: range) -> Path:
    header = {"version": 1, "since": SINCE, "until": UNTIL, "event_types": EVENT_CODES}
    lines = [orjson.dumps(header)]
    lines += [orjson.dumps({"u": f"{n:032x}", "e": e}) for n, e in enumerate(users)]
    zero = base64.b64encode(np.ones(512, dtype="<f2").tobytes()).decode()
    mert = base64.b64encode(np.ones(1024, dtype="<f2").tobytes()).decode()
    lines += [orjson.dumps({"i": i, "clap": zero, "mert": mert, "collab": None}) for i in featured]
    path = folder / "input.jsonl"
    path.write_bytes(b"\n".join(lines) + b"\n")
    return path
