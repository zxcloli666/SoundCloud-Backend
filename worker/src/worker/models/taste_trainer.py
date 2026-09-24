from __future__ import annotations

import base64
import binascii
import hashlib
import math
import time
from collections.abc import Callable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from pathlib import Path
from typing import TypeGuard

import numpy as np
import orjson
import torch
from numpy.typing import NDArray
from safetensors.torch import save_file

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

METHOD = "train"
DATASET_VERSION = 1
DIM = 128
HIDDEN = 512
CLAP_DIM = 512
MERT_DIM = 1024
COLLAB_DIM = 128
FEATURE_DIM = CLAP_DIM + MERT_DIM + COLLAB_DIM
CLAP_SLICE = slice(0, CLAP_DIM)
COLLAB_SLICE = slice(CLAP_DIM + MERT_DIM, FEATURE_DIM)
EVENT_TYPES = ("like", "like_import", "playlist_add", "full_play", "skip", "dislike")
LIKE, LIKE_IMPORT, PLAYLIST_ADD, FULL_PLAY, SKIP, DISLIKE = range(len(EVENT_TYPES))
INITIAL_TYPE_WEIGHTS = (1.0, 1.0, 1.0, 1.0, -1.0, -1.0)
REPEATED_PLAYS = 2
MIN_TIMED_POSITIVES = 5
TEST_SHARE = 0.1
MAX_EVENTS = 200
MAX_EXPLICIT_NEGATIVES = 16
MIN_ID_EVENTS = 5
SHORT_HISTORY = 10
TOP_RECALL = 50
TOP_NDCG = 20
FIT_BUDGET_SHARE = 0.75
FINE_TUNE_EPOCHS = 2
TAU_DAYS = 30.0
DAY_S = 86_400.0
LOGIT_SCALE = 20.0
LEARNING_RATE = 1e-3
WEIGHT_DECAY = 1e-4
EVAL_USERS = 256
TOWER_ROWS = 8192
MIN_NORM = 1e-6
MAX_TRACK_ID = 2**64 - 1
TOWER_SUFFIX = "-tower"
BASELINES = ("popularity", "item2vec", "content")
PREVIOUS_NONE = "none"
PREVIOUS_UNREADABLE = "unreadable"
PREVIOUS_LOADED = "loaded"
PREVIOUS_TOO_FEW_FRESH = "too_few_fresh"
PREVIOUS_COMPARED = "compared"
STRENGTHS = ("abs_weight", "type_only")

Int64Array = NDArray[np.int64]
FloatArray = NDArray[np.float32]


class TasteTrainerSlot:
    def __init__(self) -> None:
        self._device: torch.device | None = None

    def load(self, spec: SlotSpec) -> None:
        self._device = torch.device(spec.device)

    def warmup(self) -> None:
        device = self._loaded_device()
        tower = ItemTower(np.zeros(1, dtype=np.int64)).to(device)
        with torch.no_grad():
            tower(
                torch.zeros((1, FEATURE_DIM), device=device),
                torch.zeros(1, dtype=torch.long, device=device),
            )

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        device = self._loaded_device()
        if method != METHOD:
            raise BadInput(f"taste trainer has no method {method!r}")
        return {}, train(TrainParams.parse(args), device).to_result()

    def unload(self) -> None:
        self._device = None

    def _loaded_device(self) -> torch.device:
        if self._device is None:
            raise RuntimeError("taste trainer slot is not loaded")
        return self._device


@dataclass(frozen=True)
class TrainParams:
    input_path: Path
    artifact_path: Path
    tower_path: Path
    epochs: int
    batch_size: int
    negatives: int
    seed: int
    min_users: int
    budget_s: float
    trained_at: int
    previous_path: Path | None = None

    @classmethod
    def parse(cls, args: Mapping[str, object]) -> TrainParams:
        previous = args.get("previous_path")
        if previous is not None and (not isinstance(previous, str) or not previous):
            raise BadInput("previous_path must be a non-empty string or absent")
        return cls(
            input_path=Path(text_arg(args, "input_path")),
            artifact_path=Path(text_arg(args, "artifact_path")),
            tower_path=Path(text_arg(args, "tower_path")),
            epochs=positive_arg(args, "epochs"),
            batch_size=positive_arg(args, "batch_size"),
            negatives=positive_arg(args, "negatives"),
            seed=positive_arg(args, "seed"),
            min_users=positive_arg(args, "min_users"),
            budget_s=float(positive_arg(args, "budget_s")),
            trained_at=positive_arg(args, "trained_at"),
            previous_path=None if previous is None else Path(previous),
        )


@dataclass(frozen=True)
class Scores:
    recall_at_50: float
    ndcg_at_20: float
    cold_recall_at_50: float
    coverage_at_50: float
    users: int
    cold_users: int

    def to_json(self) -> dict[str, object]:
        return {
            "recall_at_50": self.recall_at_50,
            "ndcg_at_20": self.ndcg_at_20,
            "cold_recall_at_50": self.cold_recall_at_50,
            "coverage_at_50": self.coverage_at_50,
            "users": self.users,
            "cold_users": self.cold_users,
        }


@dataclass(frozen=True)
class Comparison:
    until: int
    users: int
    model: Scores
    previous: Scores

    def to_json(self) -> dict[str, object]:
        return {
            "until": self.until,
            "users": self.users,
            "model": self.model.to_json(),
            "previous": self.previous.to_json(),
        }


@dataclass(frozen=True)
class Evaluation:
    overall: Mapping[str, Scores]
    short_history: Mapping[str, Scores]
    previous: Comparison | None = None

    def min_positives(self) -> int:
        model = self.short_history["model"]
        best = max(self.short_history[name].ndcg_at_20 for name in BASELINES)
        return SHORT_HISTORY if model.users and model.ndcg_at_20 < best else 1

    def to_json(self) -> dict[str, object]:
        result: dict[str, object] = {
            "model": self.overall["model"].to_json(),
            "baselines": {name: self.overall[name].to_json() for name in BASELINES},
            "segments": {
                "short_history": {
                    "model": self.short_history["model"].to_json(),
                    "baselines": {name: self.short_history[name].to_json() for name in BASELINES},
                }
            },
        }
        if self.previous is not None:
            result["previous"] = self.previous.to_json()
        return result


@dataclass(frozen=True)
class Training:
    users_count: int
    items_count: int
    test_users: int
    evaluated_users: int
    epochs_done: int = 0
    steps: int = 0
    budget_spent: bool = False
    previous_state: str = PREVIOUS_NONE
    version: str | None = None
    tower_object: str | None = None
    evaluation: Evaluation | None = None

    def to_result(self) -> dict[str, object]:
        result: dict[str, object] = {
            "users_count": self.users_count,
            "items_count": self.items_count,
            "test_users": self.test_users,
            "evaluated_users": self.evaluated_users,
            "epochs_done": self.epochs_done,
            "steps": self.steps,
            "budget_spent": self.budget_spent,
            "previous_state": self.previous_state,
            "trained": self.evaluation is not None,
        }
        if self.evaluation is not None:
            result["version"] = self.version
            result["tower_object"] = self.tower_object
            result["metrics"] = self.evaluation.to_json()
        return result


def train(params: TrainParams, device: torch.device) -> Training:
    dataset = read_dataset(params.input_path)
    split = Split.of(dataset, params.min_users)
    untrained = Training(
        len(dataset.users), dataset.items, split.test_users, len(split.test_profiles)
    )
    enough = min(split.test_users, len(split.test_profiles)) >= params.min_users
    if not enough or not split.past.trainable:
        return untrained
    previous, state = read_previous(params.previous_path, dataset)
    with reproducible(device, params.seed):
        return fitted(
            params, dataset, split, previous, replace(untrained, previous_state=state), device
        )


@contextmanager
def reproducible(device: torch.device, seed: int) -> Iterator[None]:
    before = torch.are_deterministic_algorithms_enabled()
    torch.manual_seed(seed)
    torch.use_deterministic_algorithms(before or device.type == "cpu")
    try:
        yield
    finally:
        torch.use_deterministic_algorithms(before)


def fitted(
    params: TrainParams,
    dataset: Dataset,
    split: Split,
    previous: PreviousModel | None,
    untrained: Training,
    device: torch.device,
) -> Training:
    started = time.monotonic()
    features = torch.from_numpy(dataset.features).to(device)
    full_slots = split.full.id_slots()
    model = TasteModel(np.where(split.past.id_slots() > 0, full_slots, 0), full_slots, features)
    model.to(device)
    fit = Fitter(model, split.past, params, device)
    epochs_done, steps = fit.run(params.epochs, started + FIT_BUDGET_SHARE * params.budget_s)
    if steps == 0:
        return replace(untrained, budget_spent=True)
    model.eval()
    with torch.no_grad():
        evaluator = Evaluator(split, dataset, device)
        ranker = model.ranker(model.item_vectors())
        evaluation = evaluator.evaluate({"model": ranker}, extra_baselines(split, dataset, device))
        if previous is not None:
            comparison = evaluator.compare(ranker, previous, params.min_users // 2)
            state = PREVIOUS_TOO_FEW_FRESH if comparison is None else PREVIOUS_COMPARED
            evaluation = replace(evaluation, previous=comparison)
            untrained = replace(untrained, previous_state=state)
    model.serve_every_slot()
    tuning = Fitter(model, split.full, params, device).run(
        FINE_TUNE_EPOCHS, started + params.budget_s
    )
    model.eval()
    with torch.no_grad():
        items = model.item_vectors()
        version = write_artifact(
            params, dataset, model, items, evaluation, (epochs_done, steps), tuning, split
        )
    return replace(
        untrained,
        epochs_done=epochs_done,
        steps=steps,
        version=version,
        tower_object=version + TOWER_SUFFIX,
        evaluation=evaluation,
    )


@dataclass(frozen=True)
class UserEvents:
    tracks: NDArray[np.uint64]
    kinds: NDArray[np.int8]
    times: NDArray[np.float64]
    weights: FloatArray


@dataclass(frozen=True)
class Dataset:
    since: int
    until: int
    users: Sequence[UserEvents]
    track_ids: NDArray[np.uint64]
    features: NDArray[np.float16]
    has_collab: NDArray[np.bool_]
    order: Int64Array = field(init=False)
    sorted_ids: NDArray[np.uint64] = field(init=False)

    def __post_init__(self) -> None:
        order = np.argsort(self.track_ids, kind="stable").astype(np.int64)
        object.__setattr__(self, "order", order)
        object.__setattr__(self, "sorted_ids", self.track_ids[order])

    @property
    def items(self) -> int:
        return int(self.track_ids.shape[0])

    def index_of(self, tracks: NDArray[np.uint64]) -> Int64Array:
        if self.items == 0:
            return np.full(tracks.shape, -1, dtype=np.int64)
        found = np.searchsorted(self.sorted_ids, tracks)
        clipped = np.minimum(found, self.items - 1)
        hit = self.sorted_ids[clipped] == tracks
        return np.where(hit, self.order[clipped], -1).astype(np.int64)


def read_dataset(path: Path) -> Dataset:
    try:
        with path.open("rb") as source:
            return DatasetReader(source).read()
    except (UnicodeDecodeError, orjson.JSONDecodeError) as error:
        raise BadInput(f"taste input is not valid JSON Lines: {error}") from error


class DatasetReader:
    def __init__(self, source: Iterator[bytes]) -> None:
        self.source = source
        self.codes: dict[int, int] = {}
        self.users: list[UserEvents] = []
        self.track_ids: list[int] = []
        self.rows: list[NDArray[np.float16]] = []
        self.has_collab: list[bool] = []
        self.seen_tracks: set[int] = set()

    def read(self) -> Dataset:
        lines = (line for line in self.source if line.strip())
        head = next(lines, None)
        if head is None:
            raise BadInput("taste input is empty")
        since, until = self.read_header(orjson.loads(head))
        for line in lines:
            record = orjson.loads(line)
            if not isinstance(record, dict):
                raise BadInput("every taste input line must be an object")
            if "u" in record:
                self.read_user(record)
            elif "i" in record:
                self.read_features(record)
            else:
                raise BadInput(f"unknown taste input line with keys {sorted(record)}")
        features = (
            np.stack(self.rows) if self.rows else np.zeros((0, FEATURE_DIM), dtype=np.float16)
        )
        return Dataset(
            since=since,
            until=until,
            users=self.users,
            track_ids=np.array(self.track_ids, dtype=np.uint64),
            features=features,
            has_collab=np.array(self.has_collab, dtype=np.bool_),
        )

    def read_header(self, header: object) -> tuple[int, int]:
        if not isinstance(header, dict) or header.get("version") != DATASET_VERSION:
            raise BadInput(f"taste input header must carry version {DATASET_VERSION}")
        names = header.get("event_types")
        if not isinstance(names, dict) or set(names) != set(EVENT_TYPES):
            raise BadInput(f"event_types must name exactly {list(EVENT_TYPES)}")
        for name, code in names.items():
            if isinstance(code, bool) or not isinstance(code, int):
                raise BadInput(f"event type {name} has a non-integer code")
            self.codes[code] = EVENT_TYPES.index(name)
        if len(self.codes) != len(EVENT_TYPES):
            raise BadInput("event type codes must be distinct")
        return unix_field(header, "since"), unix_field(header, "until")

    def read_user(self, record: Mapping[str, object]) -> None:
        events = record.get("e")
        if not isinstance(record.get("u"), str) or not isinstance(events, list):
            raise BadInput("a user line needs a string 'u' and an array 'e'")
        tracks = np.empty(len(events), dtype=np.uint64)
        kinds = np.empty(len(events), dtype=np.int8)
        times = np.empty(len(events), dtype=np.float64)
        weights = np.empty(len(events), dtype=np.float32)
        for position, event in enumerate(events):
            track, kind, moment, weight = self.checked_event(event)
            tracks[position] = track
            kinds[position] = kind
            times[position] = moment
            weights[position] = weight
        self.users.append(UserEvents(tracks, kinds, times, weights))

    def checked_event(self, event: object) -> tuple[int, int, float, float]:
        if not isinstance(event, list) or len(event) != 4:
            raise BadInput("every event must be [track_id, type, unix_s, weight]")
        track, code, moment, weight = event
        if not is_integer(track) or not 0 <= track <= MAX_TRACK_ID:
            raise BadInput(f"track id {track!r} is not an unsigned 64-bit integer")
        if not is_integer(code) or code not in self.codes:
            raise BadInput(f"event type {code!r} is not declared in the header")
        if moment is not None and not is_integer(moment):
            raise BadInput(f"event time {moment!r} is not an integer or null")
        if isinstance(weight, bool) or not isinstance(weight, int | float):
            raise BadInput(f"event weight {weight!r} is not a number")
        if not math.isfinite(weight):
            raise BadInput("event weight must be finite")
        return track, self.codes[code], math.nan if moment is None else float(moment), weight

    def read_features(self, record: Mapping[str, object]) -> None:
        track = record.get("i")
        if not is_integer(track) or not 0 <= track <= MAX_TRACK_ID:
            raise BadInput(f"feature track id {track!r} is not an unsigned 64-bit integer")
        if track in self.seen_tracks:
            raise BadInput(f"track {track} has two feature lines")
        self.seen_tracks.add(track)
        collab = record.get("collab")
        row = np.zeros(FEATURE_DIM, dtype=np.float16)
        row[CLAP_SLICE] = decoded(record.get("clap"), CLAP_DIM, "clap")
        row[CLAP_DIM : CLAP_DIM + MERT_DIM] = decoded(record.get("mert"), MERT_DIM, "mert")
        if collab is not None:
            row[COLLAB_SLICE] = decoded(collab, COLLAB_DIM, "collab")
        self.track_ids.append(track)
        self.rows.append(row)
        self.has_collab.append(collab is not None)


def decoded(value: object, dim: int, name: str) -> NDArray[np.float16]:
    if not isinstance(value, str):
        raise BadInput(f"{name} must be a base64 string")
    try:
        raw = base64.b64decode(value, validate=True)
    except binascii.Error as error:
        raise BadInput(f"{name} is not valid base64: {error}") from error
    if len(raw) != dim * 2:
        raise BadInput(f"{name} holds {len(raw)} bytes, expected fp16[{dim}]")
    vector = np.frombuffer(raw, dtype="<f2").astype(np.float16)
    if not np.all(np.isfinite(vector)):
        raise BadInput(f"{name} has non-finite values")
    return vector


@dataclass(frozen=True)
class Profile:
    items: Int64Array
    kinds: NDArray[np.int8]
    ages: FloatArray
    untimed: NDArray[np.bool_]
    strengths: FloatArray
    weights: FloatArray
    positives: Int64Array
    negatives: Int64Array
    seen: Int64Array
    test: Int64Array
    history_positives: int
    kept_items: Int64Array
    reference: float


@dataclass(frozen=True)
class HeldOut:
    profile: Profile
    test_tracks: NDArray[np.uint64]


class Corpus:
    def __init__(self, profiles: list[Profile], items: int) -> None:
        self.profiles = profiles
        self.items = items
        kept = [profile.kept_items for profile in profiles]
        merged = np.concatenate(kept) if kept else np.zeros(0, dtype=np.int64)
        self.event_counts = np.bincount(merged, minlength=items).astype(np.int64)
        positives = [profile.positives for profile in profiles]
        joined = np.concatenate(positives) if positives else np.zeros(0, dtype=np.int64)
        self.positive_users = np.bincount(joined, minlength=items).astype(np.int64)

    @property
    def trainable(self) -> bool:
        return any(profile.positives.size and profile.items.size for profile in self.profiles)

    def id_slots(self) -> Int64Array:
        eligible = self.event_counts >= MIN_ID_EVENTS
        slots = np.zeros(self.items, dtype=np.int64)
        slots[eligible] = np.arange(1, int(eligible.sum()) + 1)
        return slots

    def cold(self) -> NDArray[np.bool_]:
        return np.asarray(self.event_counts == 0)


@dataclass
class Split:
    past: Corpus
    full: Corpus
    test_profiles: list[Profile]
    test_users: int
    horizon: float

    @classmethod
    def of(cls, dataset: Dataset, min_users: int) -> Split:
        held: list[HeldOut | None] = []
        test_users = 0
        for user in dataset.users:
            one, timed = held_out(user, dataset)
            test_users += int(timed >= MIN_TIMED_POSITIVES)
            held.append(one if one is not None and one.profile.test.size else None)
        references = [one.profile.reference for one in held if one is not None]
        horizon = horizon_of(references, min_users, float(dataset.until))
        chosen = [
            one if one is not None and one.profile.reference >= horizon else None for one in held
        ]
        past = [
            profile_of(user, before(user, horizon, one), horizon, (), dataset)
            for user, one in zip(dataset.users, chosen, strict=True)
        ]
        full = [
            profile_of(user, everything(user), float(dataset.until), (), dataset)
            for user in dataset.users
        ]
        return cls(
            past=Corpus(past, dataset.items),
            full=Corpus(full, dataset.items),
            test_profiles=[one.profile for one in chosen if one is not None],
            test_users=test_users,
            horizon=horizon,
        )


def horizon_of(references: Sequence[float], min_users: int, until: float) -> float:
    if not references:
        return until
    latest_first = sorted(references, reverse=True)
    return latest_first[min(min_users, len(latest_first)) - 1]


def everything(user: UserEvents) -> NDArray[np.bool_]:
    return np.ones(user.tracks.shape[0], dtype=np.bool_)


def before(user: UserEvents, horizon: float, held: HeldOut | None) -> NDArray[np.bool_]:
    keep = (user.times < horizon) | np.isnan(user.times)
    if held is None:
        return np.asarray(keep)
    return np.asarray(keep & ~np.isin(user.tracks, held.test_tracks))


def held_out(user: UserEvents, dataset: Dataset) -> tuple[HeldOut | None, int]:
    positives = positive_times(user.tracks, user.kinds, user.times)
    timed = sorted((moment, track) for track, moment in positives.items() if math.isfinite(moment))
    if len(timed) < MIN_TIMED_POSITIVES:
        return None, len(timed)
    held = timed[-max(1, math.ceil(TEST_SHARE * len(timed))) :]
    reference = held[0][0]
    test_tracks = np.array([track for _, track in held], dtype=np.uint64)
    in_test = np.isin(user.tracks, test_tracks)
    keep = ((user.times < reference) | np.isnan(user.times)) & ~in_test
    profile = profile_of(user, np.asarray(keep), reference, test_tracks, dataset)
    return HeldOut(profile, test_tracks), len(timed)


def profile_of(
    user: UserEvents,
    keep: NDArray[np.bool_],
    reference: float,
    test_tracks: Sequence[int] | NDArray[np.uint64],
    dataset: Dataset,
) -> Profile:
    tracks, kinds, times = user.tracks[keep], user.kinds[keep], user.times[keep]
    weights = user.weights[keep]
    items = dataset.index_of(tracks)
    history = positive_times(tracks, kinds, times)
    history_items = dataset.index_of(np.array(list(history), dtype=np.uint64))
    positive_items = np.unique(history_items[history_items >= 0])
    featured = items >= 0
    seen = np.unique(items[featured])
    test_items = dataset.index_of(np.asarray(test_tracks, dtype=np.uint64))
    test_items = np.setdiff1d(np.unique(test_items[test_items >= 0]), seen)
    order = np.argsort(np.where(np.isnan(times), -np.inf, times), kind="stable")
    order = order[featured[order]][-MAX_EVENTS:]
    ages = np.where(np.isnan(times[order]), 0.0, (reference - times[order]) / DAY_S)
    return Profile(
        items=items[order],
        kinds=kinds[order],
        ages=np.maximum(ages, 0.0).astype(np.float32),
        untimed=np.isnan(times[order]),
        strengths=np.abs(weights[order]).astype(np.float32),
        weights=weights[order].astype(np.float32),
        positives=positive_items,
        negatives=recent_negatives(items, kinds, times, weights),
        seen=seen,
        test=test_items,
        history_positives=len(history),
        kept_items=items[featured],
        reference=reference,
    )


def recent_negatives(
    items: Int64Array, kinds: NDArray[np.int8], times: NDArray[np.float64], weights: FloatArray
) -> Int64Array:
    explicit = ((kinds == DISLIKE) | ((kinds == SKIP) & (weights < 0))) & (items >= 0)
    moments = np.where(np.isnan(times[explicit]), -np.inf, times[explicit])
    newest_first = items[explicit][np.argsort(-moments, kind="stable")]
    _, first_seen = np.unique(newest_first, return_index=True)
    return np.asarray(newest_first[np.sort(first_seen)[:MAX_EXPLICIT_NEGATIVES]], dtype=np.int64)


def positive_times(
    tracks: NDArray[np.uint64], kinds: NDArray[np.int8], times: NDArray[np.float64]
) -> dict[int, float]:
    disliked = {int(track) for track in tracks[kinds == DISLIKE]}
    moments: dict[int, float] = {}
    plays: dict[int, list[float]] = {}
    imported: set[int] = set()
    for track_value, kind, moment in zip(
        tracks.tolist(), kinds.tolist(), times.tolist(), strict=True
    ):
        track = int(track_value)
        if kind == LIKE_IMPORT or (kind in (LIKE, PLAYLIST_ADD) and math.isnan(moment)):
            imported.add(track)
        elif kind in (LIKE, PLAYLIST_ADD):
            moments[track] = min(moment, moments.get(track, math.inf))
        elif kind == FULL_PLAY:
            plays.setdefault(track, []).append(moment)
    for track, played in plays.items():
        if len(played) >= REPEATED_PLAYS:
            repeated = sorted(played)[REPEATED_PLAYS - 1]
            moments[track] = min(repeated, moments.get(track, math.inf))
    for track in imported:
        moments.setdefault(track, math.nan)
    return {track: moment for track, moment in moments.items() if track not in disliked}


class ItemTower(torch.nn.Module):
    def __init__(self, id_slots: Int64Array) -> None:
        super().__init__()
        self.hidden = torch.nn.Linear(FEATURE_DIM, HIDDEN)
        self.out = torch.nn.Linear(HIDDEN, DIM)
        self.ids = torch.nn.Embedding(int(id_slots.max(initial=0)) + 1, DIM, padding_idx=0)
        torch.nn.init.zeros_(self.ids.weight)

    def forward(self, features: torch.Tensor, slots: torch.Tensor) -> torch.Tensor:
        content = self.out(torch.relu(self.hidden(features)))
        return torch.nn.functional.normalize(content + self.ids(slots), dim=-1)


class Pooling(torch.nn.Module):
    def __init__(self) -> None:
        super().__init__()
        self.type_weights = torch.nn.Parameter(torch.tensor(INITIAL_TYPE_WEIGHTS))
        self.log_tau = torch.nn.Parameter(torch.tensor(math.log(TAU_DAYS)))

    def forward(self, context: Context, vectors: torch.Tensor, mask: torch.Tensor) -> torch.Tensor:
        decay = torch.where(context.untimed, 1.0, torch.exp(-context.ages / self.log_tau.exp()))
        coefficients = self.type_weights[context.kinds] * context.strengths * decay * mask
        return torch.einsum("bl,bld->bd", coefficients, vectors)

    def to_json(self, min_positives: int) -> dict[str, object]:
        weights = self.type_weights.detach().cpu().tolist()
        return {
            "w": dict(zip(EVENT_TYPES, weights, strict=True)),
            "tau_days": float(self.log_tau.detach().exp().item()),
            "max_events": MAX_EVENTS,
            "strength": "abs_weight",
            "decay": "exp_age_over_tau",
            "untimed_decay": 1.0,
            "min_positives": min_positives,
        }


class TasteModel(torch.nn.Module):
    def __init__(
        self, fitted_slots: Int64Array, served_slots: Int64Array, features: torch.Tensor
    ) -> None:
        super().__init__()
        self.tower = ItemTower(served_slots)
        self.pooling = Pooling()
        self.features = features
        self.slots = torch.from_numpy(fitted_slots).to(features.device)
        self.served_slots = torch.from_numpy(served_slots).to(features.device)

    def serve_every_slot(self) -> None:
        self.slots = self.served_slots

    def items_of(self, rows: torch.Tensor) -> torch.Tensor:
        vectors: torch.Tensor = self.tower(self.features[rows].float(), self.slots[rows])
        return vectors

    def item_vectors(self) -> torch.Tensor:
        count = self.features.shape[0]
        device = self.features.device
        chunks = [
            self.items_of(torch.arange(start, min(start + TOWER_ROWS, count), device=device))
            for start in range(0, count, TOWER_ROWS)
        ]
        return torch.cat(chunks) if chunks else torch.zeros((0, DIM), device=device)

    def ranker(self, items: torch.Tensor) -> Ranker:
        def rank(context: Context) -> torch.Tensor:
            vectors = items[context.items.clamp_min(0)]
            users = unit(self.pooling(context, vectors, context.valid.float()))
            scores = users @ items.T
            scores[users.norm(dim=-1) < MIN_NORM] = -math.inf
            return scores

        return rank


@dataclass(frozen=True)
class Context:
    items: torch.Tensor
    kinds: torch.Tensor
    ages: torch.Tensor
    untimed: torch.Tensor
    strengths: torch.Tensor
    weights: torch.Tensor
    valid: torch.Tensor

    @classmethod
    def of(cls, profiles: Sequence[Profile], device: torch.device) -> Context:
        width = max((profile.items.size for profile in profiles), default=0)
        rows = len(profiles)
        items = np.full((rows, width), -1, dtype=np.int64)
        kinds = np.zeros((rows, width), dtype=np.int64)
        ages = np.zeros((rows, width), dtype=np.float32)
        untimed = np.zeros((rows, width), dtype=np.bool_)
        strengths = np.zeros((rows, width), dtype=np.float32)
        weights = np.zeros((rows, width), dtype=np.float32)
        for row, profile in enumerate(profiles):
            length = profile.items.size
            items[row, :length] = profile.items
            kinds[row, :length] = profile.kinds
            ages[row, :length] = profile.ages
            untimed[row, :length] = profile.untimed
            strengths[row, :length] = profile.strengths
            weights[row, :length] = profile.weights
        return cls(
            items=torch.from_numpy(items).to(device),
            kinds=torch.from_numpy(kinds).to(device),
            ages=torch.from_numpy(ages).to(device),
            untimed=torch.from_numpy(untimed).to(device),
            strengths=torch.from_numpy(strengths).to(device),
            weights=torch.from_numpy(weights).to(device),
            valid=torch.from_numpy(items >= 0).to(device),
        )

    def rows(self, index: torch.Tensor) -> Context:
        return Context(
            items=self.items[index],
            kinds=self.kinds[index],
            ages=self.ages[index],
            untimed=self.untimed[index],
            strengths=self.strengths[index],
            weights=self.weights[index],
            valid=self.valid[index],
        )


Ranker = Callable[[Context], torch.Tensor]


class Fitter:
    def __init__(
        self, model: TasteModel, corpus: Corpus, params: TrainParams, device: torch.device
    ) -> None:
        self.model = model
        self.params = params
        self.device = device
        profiles = [p for p in corpus.profiles if p.positives.size and p.items.size]
        self.context = Context.of(profiles, device)
        self.negatives = padded([p.negatives for p in profiles], device)
        users = np.concatenate(
            [np.full(p.positives.size, row, dtype=np.int64) for row, p in enumerate(profiles)]
        )
        targets = np.concatenate([p.positives for p in profiles])
        self.users = torch.from_numpy(users).to(device)
        self.targets = torch.from_numpy(targets).to(device)
        frequency = np.bincount(targets, minlength=corpus.items) / max(1, targets.size)
        self.log_frequency = torch.from_numpy(np.log(np.maximum(frequency, 1e-12))).to(device)
        self.items = corpus.items
        self.generator = torch.Generator(device="cpu").manual_seed(params.seed)
        self.optimizer = torch.optim.AdamW(
            model.parameters(), lr=LEARNING_RATE, weight_decay=WEIGHT_DECAY
        )

    def run(self, epochs: int, deadline: float) -> tuple[int, int]:
        self.model.train()
        steps = 0
        for epoch in range(epochs):
            order = torch.randperm(self.targets.shape[0], generator=self.generator)
            for start in range(0, order.shape[0], self.params.batch_size):
                if time.monotonic() >= deadline:
                    return epoch, steps
                self.step(order[start : start + self.params.batch_size].to(self.device))
                steps += 1
        return epochs, steps

    def step(self, batch: torch.Tensor) -> None:
        rows = self.users[batch]
        targets = self.targets[batch]
        context = self.context.rows(rows)
        context_mask = context.valid & (context.items != targets[:, None])
        explicit = self.negatives[0][rows]
        explicit_mask = self.negatives[1][rows] & (explicit != targets[:, None])
        sampled = torch.randint(self.items, (self.params.negatives,), generator=self.generator).to(
            self.device
        )
        wanted = torch.cat([context.items[context_mask], targets, sampled, explicit[explicit_mask]])
        unique, inverse = torch.unique(wanted, return_inverse=True)
        vectors = self.model.items_of(unique)
        positions = inverse.split(
            [int(context_mask.sum()), targets.shape[0], sampled.shape[0], int(explicit_mask.sum())]
        )
        context_rows = torch.zeros_like(context.items)
        context_rows[context_mask] = positions[0]
        explicit_rows = torch.zeros_like(explicit)
        explicit_rows[explicit_mask] = positions[3]
        pooled = self.model.pooling(context, vectors[context_rows], context_mask.float())
        norms = pooled.norm(dim=-1)
        live = norms > MIN_NORM
        if not bool(live.any()):
            return
        users = pooled / norms.clamp_min(MIN_NORM)[:, None]
        logits = self.logits(
            users, vectors, positions, targets, sampled, (explicit_rows, explicit_mask)
        )
        labels = torch.arange(targets.shape[0], device=self.device)
        loss = torch.nn.functional.cross_entropy(logits[live], labels[live])
        self.optimizer.zero_grad(set_to_none=True)
        torch.autograd.backward(loss)
        self.optimizer.step()

    def logits(
        self,
        users: torch.Tensor,
        vectors: torch.Tensor,
        positions: Sequence[torch.Tensor],
        targets: torch.Tensor,
        sampled: torch.Tensor,
        explicit_negatives: tuple[torch.Tensor, torch.Tensor],
    ) -> torch.Tensor:
        explicit_rows, explicit_mask = explicit_negatives
        batch = targets.shape[0]
        in_batch = LOGIT_SCALE * users @ vectors[positions[1]].T
        in_batch = in_batch - (self.log_frequency[targets] + math.log(batch))[None, :]
        collisions = (targets[:, None] == targets[None, :]) & ~torch.eye(
            batch, dtype=torch.bool, device=self.device
        )
        in_batch = in_batch.masked_fill(collisions, -math.inf)
        uniform = LOGIT_SCALE * users @ vectors[positions[2]].T
        uniform = uniform - math.log(sampled.shape[0] / self.items)
        uniform = uniform.masked_fill(targets[:, None] == sampled[None, :], -math.inf)
        explicit = LOGIT_SCALE * torch.einsum("bd,bkd->bk", users, vectors[explicit_rows])
        explicit = explicit.masked_fill(~explicit_mask, -math.inf)
        return torch.cat([in_batch, uniform, explicit], dim=1)


def padded(rows: Sequence[Int64Array], device: torch.device) -> tuple[torch.Tensor, torch.Tensor]:
    width = max((row.size for row in rows), default=0)
    values = np.zeros((len(rows), width), dtype=np.int64)
    mask = np.zeros((len(rows), width), dtype=np.bool_)
    for index, row in enumerate(rows):
        values[index, : row.size] = row
        mask[index, : row.size] = True
    return torch.from_numpy(values).to(device), torch.from_numpy(mask).to(device)


def extra_baselines(split: Split, dataset: Dataset, device: torch.device) -> dict[str, Ranker]:
    popularity = torch.from_numpy(split.past.positive_users.astype(np.float32)).to(device)
    collab = torch.from_numpy(dataset.features[:, COLLAB_SLICE].astype(np.float32)).to(device)
    clap = torch.from_numpy(dataset.features[:, CLAP_SLICE].astype(np.float32)).to(device)
    has_collab = torch.from_numpy(dataset.has_collab).to(device)
    return {
        "popularity": lambda context: popularity.expand(context.items.shape[0], -1).clone(),
        "item2vec": pooled_ranker(unit(collab), has_collab, api_weights),
        "content": pooled_ranker(unit(clap), torch.ones_like(has_collab), api_weights),
    }


def api_weights(context: Context) -> torch.Tensor:
    decay = torch.where(context.untimed, 1.0, torch.exp(-context.ages / TAU_DAYS))
    return context.weights * decay


def pooled_ranker(
    vectors: torch.Tensor, present: torch.Tensor, weigh: Callable[[Context], torch.Tensor]
) -> Ranker:
    def rank(context: Context) -> torch.Tensor:
        rows = context.items.clamp_min(0)
        usable = context.valid & present[rows]
        coefficients = weigh(context) * usable
        users = unit(torch.einsum("bl,bld->bd", coefficients, vectors[rows]))
        scores = users @ vectors.T
        scores[:, ~present] = -math.inf
        scores[users.norm(dim=-1) < MIN_NORM] = -math.inf
        return scores

    return rank


def unit(rows: torch.Tensor) -> torch.Tensor:
    return torch.nn.functional.normalize(rows, dim=-1, eps=MIN_NORM)


@dataclass(frozen=True)
class PreviousModel:
    until: int
    vectors: FloatArray
    present: NDArray[np.bool_]
    type_weights: FloatArray
    tau_days: float
    by_weight: bool

    def ranker(self, device: torch.device) -> Ranker:
        vectors = torch.from_numpy(self.vectors).to(device)
        present = torch.from_numpy(self.present).to(device)
        type_weights = torch.from_numpy(self.type_weights).to(device)

        def weigh(context: Context) -> torch.Tensor:
            decay = torch.where(context.untimed, 1.0, torch.exp(-context.ages / self.tau_days))
            strengths = context.strengths if self.by_weight else torch.ones_like(decay)
            return type_weights[context.kinds] * strengths * decay

        return pooled_ranker(vectors, present, weigh)


def read_previous(path: Path | None, dataset: Dataset) -> tuple[PreviousModel | None, str]:
    if path is None:
        return None, PREVIOUS_NONE
    try:
        return previous_model(orjson.loads(path.read_bytes()), dataset), PREVIOUS_LOADED
    except (OSError, orjson.JSONDecodeError, KeyError, TypeError, ValueError, OverflowError):
        return None, PREVIOUS_UNREADABLE


def previous_model(document: object, dataset: Dataset) -> PreviousModel:
    if not isinstance(document, dict):
        raise ValueError("previous artifact is not an object")
    pooling = document["pooling"]
    data = document["data"]
    items = document["items"]
    if not isinstance(pooling, dict) or not isinstance(data, dict) or not isinstance(items, list):
        raise ValueError("previous artifact lacks pooling, data or items")
    weights = pooling["w"]
    type_weights = np.array([float(weights[name]) for name in EVENT_TYPES], dtype=np.float32)
    tau_days = float(pooling["tau_days"])
    strength = pooling.get("strength", "type_only")
    until = data["until"]
    ids = np.array([item["id"] for item in items], dtype=np.uint64)
    vectors = np.array([item["vec"] for item in items], dtype=np.float32).reshape(len(items), DIM)
    valid = (
        strength in STRENGTHS
        and is_integer(until)
        and math.isfinite(tau_days)
        and tau_days > 0
        and bool(np.all(np.isfinite(type_weights)))
        and bool(np.all(np.isfinite(vectors)))
    )
    if not valid:
        raise ValueError("previous artifact has an unusable pooling or vectors")
    rows = dataset.index_of(ids)
    known = rows >= 0
    aligned = np.zeros((dataset.items, DIM), dtype=np.float32)
    aligned[rows[known]] = vectors[known]
    present = np.zeros(dataset.items, dtype=np.bool_)
    present[rows[known]] = True
    return PreviousModel(
        until=int(until),
        vectors=aligned,
        present=present,
        type_weights=type_weights,
        tau_days=tau_days,
        by_weight=strength == "abs_weight",
    )


class Evaluator:
    def __init__(self, split: Split, dataset: Dataset, device: torch.device) -> None:
        self.profiles = split.test_profiles
        self.cold = split.past.cold()
        self.items = dataset.items
        self.device = device

    def evaluate(self, model: Mapping[str, Ranker], baselines: Mapping[str, Ranker]) -> Evaluation:
        short = [profile for profile in self.profiles if profile.history_positives < SHORT_HISTORY]
        rankers = {**model, **baselines}
        return Evaluation(
            overall=self.scores(self.profiles, rankers),
            short_history=self.scores(short, rankers),
        )

    def compare(self, model: Ranker, previous: PreviousModel, min_users: int) -> Comparison | None:
        fresh = [profile for profile in self.profiles if profile.reference >= previous.until]
        if len(fresh) < max(1, min_users):
            return None
        scores = self.scores(fresh, {"model": model, "previous": previous.ranker(self.device)})
        return Comparison(
            until=previous.until,
            users=scores["model"].users,
            model=scores["model"],
            previous=scores["previous"],
        )

    def scores(
        self, profiles: Sequence[Profile], rankers: Mapping[str, Ranker]
    ) -> dict[str, Scores]:
        tallies = {name: Tally() for name in rankers}
        for start in range(0, len(profiles), EVAL_USERS):
            chunk = profiles[start : start + EVAL_USERS]
            context = Context.of(chunk, self.device)
            for name, ranker in rankers.items():
                top = self.top(ranker(context), chunk)
                for profile, recommended in zip(chunk, top, strict=True):
                    tallies[name].add(recommended, profile.test, self.cold)
        return {name: tally.scores(self.items) for name, tally in tallies.items()}

    def top(self, scores: torch.Tensor, chunk: Sequence[Profile]) -> list[Int64Array]:
        for row, profile in enumerate(chunk):
            if profile.seen.size:
                scores[row, torch.from_numpy(profile.seen).to(self.device)] = -math.inf
        values, indices = torch.topk(scores, min(TOP_RECALL, self.items), dim=1)
        finite = torch.isfinite(values).cpu().numpy()
        picked = indices.cpu().numpy()
        return [picked[row][finite[row]] for row in range(len(chunk))]


@dataclass
class Tally:
    users: int = 0
    recall: float = 0.0
    ndcg: float = 0.0
    cold_users: int = 0
    cold_recall: float = 0.0
    covered: set[int] = field(default_factory=set)

    def add(self, recommended: Int64Array, truth: Int64Array, cold: NDArray[np.bool_]) -> None:
        if truth.size == 0:
            return
        self.users += 1
        hits = np.isin(recommended, truth)
        self.recall += float(hits.sum()) / truth.size
        gains = hits[:TOP_NDCG] / np.log2(np.arange(2, min(TOP_NDCG, hits.size) + 2))
        ideal = 1.0 / np.log2(np.arange(2, min(TOP_NDCG, truth.size) + 2))
        self.ndcg += float(gains.sum() / ideal.sum())
        cold_truth = truth[cold[truth]]
        if cold_truth.size:
            self.cold_users += 1
            self.cold_recall += float(np.isin(recommended, cold_truth).sum()) / cold_truth.size
        self.covered.update(recommended.tolist())

    def scores(self, catalog: int) -> Scores:
        return Scores(
            recall_at_50=self.recall / self.users if self.users else 0.0,
            ndcg_at_20=self.ndcg / self.users if self.users else 0.0,
            cold_recall_at_50=self.cold_recall / self.cold_users if self.cold_users else 0.0,
            coverage_at_50=len(self.covered) / catalog if catalog else 0.0,
            users=self.users,
            cold_users=self.cold_users,
        )


def write_artifact(
    params: TrainParams,
    dataset: Dataset,
    model: TasteModel,
    items: torch.Tensor,
    evaluation: Evaluation,
    fit: tuple[int, int],
    tuning: tuple[int, int],
    split: Split,
) -> str:
    tower = model.tower
    save_file(
        {
            "hidden.weight": tower.hidden.weight.detach().float().cpu().contiguous(),
            "hidden.bias": tower.hidden.bias.detach().float().cpu().contiguous(),
            "out.weight": tower.out.weight.detach().float().cpu().contiguous(),
            "out.bias": tower.out.bias.detach().float().cpu().contiguous(),
        },
        str(params.tower_path),
    )
    vectors = np.ascontiguousarray(items.float().cpu().numpy(), dtype=np.float32)
    pooling = model.pooling.to_json(evaluation.min_positives())
    digest = hashlib.sha256(params.tower_path.read_bytes())
    digest.update(vectors.tobytes())
    digest.update(orjson.dumps(pooling, option=orjson.OPT_SORT_KEYS))
    trained_at = datetime.fromtimestamp(params.trained_at, UTC)
    version = f"taste-{trained_at:%Y%m%d%H%M}-{digest.hexdigest()[:8]}"
    head = {
        "version": version,
        "trained_at": trained_at.isoformat().replace("+00:00", "Z"),
        "dim": DIM,
        "pooling": pooling,
        "metrics": {
            **evaluation.to_json(),
            "epochs_done": fit[0],
            "steps": fit[1],
            "fine_tune": {"epochs_done": tuning[0], "steps": tuning[1]},
            "horizon": int(split.horizon),
            "evaluated_users": len(split.test_profiles),
        },
        "tower": {
            "object": version + TOWER_SUFFIX,
            "format": "safetensors",
            "inputs": ["clap", "mert", "collab"],
            "missing_collab": "zeros",
            "layers": ["hidden", "relu", "out", "l2"],
        },
        "data": {"since": dataset.since, "until": dataset.until},
    }
    with params.artifact_path.open("wb") as sink:
        sink.write(orjson.dumps(head)[:-1] + b',"items":[')
        for row, track in enumerate(dataset.track_ids.tolist()):
            if row:
                sink.write(b",")
            point = {"id": int(track), "vec": vectors[row]}
            sink.write(orjson.dumps(point, option=orjson.OPT_SERIALIZE_NUMPY))
        sink.write(b"]}")
    return version


def unix_field(header: Mapping[str, object], key: str) -> int:
    value = header.get(key)
    if not is_integer(value):
        raise BadInput(f"header {key} must be an integer unix time")
    return int(value)


def is_integer(value: object) -> TypeGuard[int]:
    return isinstance(value, int) and not isinstance(value, bool)


def text_arg(args: Mapping[str, object], key: str) -> str:
    value = args.get(key)
    if not isinstance(value, str) or not value:
        raise BadInput(f"{key} must be a non-empty string")
    return value


def positive_arg(args: Mapping[str, object], key: str) -> int:
    value = args.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise BadInput(f"{key} must be an integer >= 1")
    return value
