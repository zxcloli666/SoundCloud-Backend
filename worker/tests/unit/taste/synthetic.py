from __future__ import annotations

import base64
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

import numpy as np
import orjson
from numpy.typing import NDArray

EVENT_CODES = {
    "like": 0,
    "like_import": 1,
    "playlist_add": 2,
    "full_play": 3,
    "skip": 4,
    "dislike": 5,
}
DAY_S = 86_400
UNTIL = 1_790_000_000
SINCE = UNTIL - 180 * DAY_S
FIRST_TRACK_ID = 100_000
CLAP_DIM = 512
MERT_DIM = 1024
COLLAB_DIM = 128

Matrix = NDArray[np.float64]
Event = list[int | float | None]


class Mode(StrEnum):
    TASTE = "taste"
    POPULARITY = "popularity"
    IMPORTS_ONLY = "imports_only"


@dataclass(frozen=True)
class World:
    users: int = 600
    tracks: int = 2400
    tastes: int = 12
    seed: int = 7
    mode: Mode = Mode.TASTE
    clap_noise: float = 1.6
    mert_noise: float = 0.9
    collab_noise: float = 1.2
    collab_share: float = 0.7
    featureless_tracks: int = 40
    zipf: float = 1.1
    import_share: float = 0.15


def write_input(path: Path, world: World) -> Path:
    path.write_bytes(synthetic_input(world))
    return path


def synthetic_input(world: World) -> bytes:
    rng = np.random.default_rng(world.seed)
    catalog = Catalog.of(world, rng)
    users = [user_line(world, catalog, rng, index) for index in range(world.users)]
    header = {
        "version": 1,
        "since": SINCE,
        "until": UNTIL,
        "event_types": EVENT_CODES,
        "users": len(users),
        "items": world.tracks,
    }
    lines = [orjson.dumps(header), *users, *catalog.feature_lines(world, rng)]
    return b"\n".join(lines) + b"\n"


@dataclass(frozen=True)
class Catalog:
    ids: NDArray[np.int64]
    tastes: NDArray[np.int64]
    popularity: Matrix
    clap_centers: Matrix
    mert_centers: Matrix
    collab_centers: Matrix
    featureless: NDArray[np.int64]

    @classmethod
    def of(cls, world: World, rng: np.random.Generator) -> Catalog:
        tastes = rng.integers(world.tastes, size=world.tracks)
        ranks = rng.permutation(world.tracks).astype(np.float64)
        popularity = 1.0 / np.power(ranks + 1.0, world.zipf)
        ids = FIRST_TRACK_ID + 7 * np.arange(world.tracks, dtype=np.int64)
        featureless = FIRST_TRACK_ID + 7 * np.arange(
            world.tracks, world.tracks + world.featureless_tracks, dtype=np.int64
        )
        return cls(
            ids=ids,
            tastes=tastes,
            popularity=popularity,
            clap_centers=unit(rng.standard_normal((world.tastes, CLAP_DIM))),
            mert_centers=unit(rng.standard_normal((world.tastes, MERT_DIM))),
            collab_centers=unit(rng.standard_normal((world.tastes, COLLAB_DIM))),
            featureless=featureless,
        )

    def pick(self, rng: np.random.Generator, taste: int | None, taken: set[int]) -> int:
        pool = np.arange(self.ids.size) if taste is None else np.flatnonzero(self.tastes == taste)
        weights = self.popularity[pool].copy()
        weights[np.isin(pool, list(taken))] = 0.0
        if weights.sum() == 0.0:
            return self.pick(rng, None, taken)
        return int(rng.choice(pool, p=weights / weights.sum()))

    def outside(self, rng: np.random.Generator, liked: set[int]) -> int:
        pool = np.flatnonzero(~np.isin(self.tastes, list(liked)))
        return int(rng.choice(pool))

    def feature_lines(self, world: World, rng: np.random.Generator) -> list[bytes]:
        lines = []
        for row, track in enumerate(self.ids.tolist()):
            taste = int(self.tastes[row])
            collab = None
            if rng.random() < world.collab_share:
                collab = encoded(noisy(self.collab_centers[taste], world.collab_noise, rng))
            record = {
                "i": track,
                "clap": encoded(noisy(self.clap_centers[taste], world.clap_noise, rng)),
                "mert": encoded(noisy(self.mert_centers[taste], world.mert_noise, rng)),
                "collab": collab,
            }
            lines.append(orjson.dumps(record))
        return lines


def user_line(world: World, catalog: Catalog, rng: np.random.Generator, index: int) -> bytes:
    liked = [int(t) for t in rng.choice(world.tastes, size=rng.integers(1, 4), replace=False)]
    mixture = rng.dirichlet(np.ones(len(liked)))
    moment = SINCE + int(rng.integers(0, 60 * DAY_S))
    count = int(rng.integers(6, 36))
    gaps = rng.exponential(1.0, count)
    gaps *= (UNTIL - DAY_S - moment) * rng.uniform(0.7, 1.0) / gaps.sum()
    events: list[Event] = []
    taken: set[int] = set()
    for gap in gaps.tolist():
        moment += int(gap)
        taste = None if world.mode is Mode.POPULARITY else liked[rng.choice(len(liked), p=mixture)]
        row = catalog.pick(rng, taste, taken)
        taken.add(row)
        events += positive_events(world, rng, int(catalog.ids[row]), min(moment, UNTIL - 1))
    for _ in range(int(rng.integers(3, 10))):
        noise_at = int(rng.integers(SINCE, UNTIL - DAY_S))
        skipped = int(catalog.ids[catalog.outside(rng, set(liked))])
        played = int(catalog.ids[catalog.pick(rng, None, set())])
        events.append([skipped, EVENT_CODES["skip"], noise_at, float(rng.choice([-0.8, -0.3]))])
        events.append([played, EVENT_CODES["full_play"], noise_at + 5, 0.3])
    for _ in range(2):
        disliked = int(catalog.ids[catalog.outside(rng, set(liked))])
        events.append([disliked, EVENT_CODES["dislike"], int(rng.integers(SINCE, UNTIL)), -1.0])
    if rng.random() < 0.3:
        unknown = int(rng.choice(catalog.featureless))
        events.append([unknown, EVENT_CODES["like"], int(rng.integers(SINCE, UNTIL)), 1.0])
    events.sort(key=event_order)
    return orjson.dumps({"u": f"{index:032x}", "e": events})


def event_order(event: Event) -> float:
    moment = event[2]
    return -1.0 if moment is None else float(moment)


def positive_events(world: World, rng: np.random.Generator, track: int, moment: int) -> list[Event]:
    if world.mode is Mode.IMPORTS_ONLY or rng.random() < world.import_share:
        return [[track, EVENT_CODES["like_import"], None, 1.0]]
    roll = rng.random()
    if roll < 0.55:
        return [[track, EVENT_CODES["like"], moment, 1.0]]
    if roll < 0.75:
        return [[track, EVENT_CODES["playlist_add"], moment, 0.9]]
    first = moment - int(rng.integers(3_600, 3 * DAY_S))
    return [
        [track, EVENT_CODES["full_play"], first, 0.3],
        [track, EVENT_CODES["full_play"], moment, 0.3],
    ]


def noisy(center: NDArray[np.float64], noise: float, rng: np.random.Generator) -> Matrix:
    jitter = rng.standard_normal(center.size) / np.sqrt(center.size)
    return unit(center + noise * jitter)


def unit(rows: Matrix) -> Matrix:
    return rows / np.linalg.norm(rows, axis=-1, keepdims=True)


def encoded(vector: Matrix) -> str:
    return base64.b64encode(vector.astype("<f2").tobytes()).decode()
