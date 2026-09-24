from __future__ import annotations

import json
import os
import re
from array import array
from collections import Counter
from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import IO

import numpy as np
import orjson
from gensim.models import Word2Vec
from numpy.typing import NDArray

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

METHOD = "train"
DIM = 128
DATASET_VERSION = 2
HOLDOUT_EVERY = 10
TOP_K = 20
MAX_EVAL_CASES = 20_000
EVAL_BATCH = 128
NS_EXPONENT = 0.5
SAMPLE = 1e-4
SEED = 1
CHUNK_CHARS = 1 << 20
MIN_SESSIONS = 2
MAX_TRACK_ID = 2**64 - 1
SESSIONS_KEY = re.compile(r'"sessions"\s*:\s*\[')
SEPARATORS = " \t\r\n,"
WARMUP_SESSIONS = [["1", "2", "3"], ["2", "3", "1"], ["3", "1", "2"]]


class Item2VecSlot:
    def __init__(self) -> None:
        self._workers: int | None = None

    def load(self, spec: SlotSpec) -> None:
        self._workers = len(os.sched_getaffinity(0))

    def warmup(self) -> None:
        Word2Vec(
            WARMUP_SESSIONS, vector_size=8, min_count=1, window=2, epochs=1, workers=1, seed=SEED
        )

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if self._workers is None:
            raise RuntimeError("item2vec slot is not loaded")
        if method != METHOD:
            raise ValueError(f"item2vec has no method {method!r}")
        params = TrainParams.parse(args, self._workers)
        return {}, train(params).to_result()

    def unload(self) -> None:
        self._workers = None


@dataclass(frozen=True)
class TrainParams:
    sessions_path: Path
    vectors_path: Path
    min_count: int
    window: int
    epochs: int
    negative: int
    workers: int

    @classmethod
    def parse(cls, args: Mapping[str, object], workers: int) -> TrainParams:
        return cls(
            sessions_path=Path(text_arg(args, "sessions_path")),
            vectors_path=Path(text_arg(args, "vectors_path")),
            min_count=positive_arg(args, "min_count"),
            window=positive_arg(args, "window"),
            epochs=positive_arg(args, "epochs"),
            negative=positive_arg(args, "negative"),
            workers=workers,
        )


@dataclass(frozen=True)
class Training:
    sessions: int
    vocab: int
    hr_at_20: float
    popularity_hr_at_20: float

    def to_result(self) -> dict[str, object]:
        return {
            "sessions": self.sessions,
            "vocab": self.vocab,
            "points_count": self.vocab,
            "hr_at_20": self.hr_at_20,
            "popularity_hr_at_20": self.popularity_hr_at_20,
        }


def train(params: TrainParams) -> Training:
    sessions = read_sessions(params.sessions_path)
    if len(sessions) < MIN_SESSIONS:
        return Training(len(sessions), 0, 0.0, 0.0)
    corpus = Split(sessions, held_out=False)
    model = Word2Vec(
        vector_size=DIM,
        sg=1,
        min_count=params.min_count,
        window=params.window,
        epochs=params.epochs,
        negative=params.negative,
        ns_exponent=NS_EXPONENT,
        sample=SAMPLE,
        workers=params.workers,
        seed=SEED,
    )
    model.build_vocab(corpus)
    if len(model.wv) == 0:
        return Training(len(sessions), 0, 0.0, 0.0)
    model.train(corpus, total_examples=model.corpus_count, epochs=model.epochs)
    keys = [str(key) for key in model.wv.index_to_key]
    vectors = unit_rows(np.asarray(model.wv.vectors, dtype=np.float32))
    hit_rate, popularity_rate = hit_rates(keys, vectors, sessions, corpus, window=params.window)
    training = Training(len(sessions), len(keys), hit_rate, popularity_rate)
    write_vectors(params.vectors_path, keys, vectors, training)
    return training


class Sessions:
    def __init__(self) -> None:
        self.items = array("Q")
        self.bounds = array("Q", [0])

    def __len__(self) -> int:
        return len(self.bounds) - 1

    def add(self, session: Sequence[int]) -> None:
        self.items.extend(session)
        self.bounds.append(len(self.items))

    def session(self, index: int) -> list[str]:
        return [str(item) for item in self.items[self.bounds[index] : self.bounds[index + 1]]]


class Split:
    def __init__(self, sessions: Sessions, *, held_out: bool) -> None:
        self.sessions = sessions
        self.held_out = held_out

    def __iter__(self) -> Iterator[list[str]]:
        for index in range(len(self.sessions)):
            if is_held_out(index) == self.held_out:
                yield self.sessions.session(index)


def is_held_out(index: int) -> bool:
    return index % HOLDOUT_EVERY == 0


def read_sessions(path: Path) -> Sessions:
    sessions = Sessions()
    try:
        with path.open(encoding="utf-8") as source:
            envelope = SessionReader(source, sessions).read()
    except (UnicodeDecodeError, json.JSONDecodeError, orjson.JSONDecodeError) as error:
        raise BadInput(f"sessions object is not valid JSON: {error}") from error
    if not isinstance(envelope, dict) or envelope.get("version") != DATASET_VERSION:
        raise BadInput(f"sessions object must be {{'version': {DATASET_VERSION}, ...}}")
    return sessions


class SessionReader:
    def __init__(self, source: IO[str], sessions: Sessions) -> None:
        self.source = source
        self.sessions = sessions
        self.decoder = json.JSONDecoder()
        self.buffer = ""
        self.position = 0

    def read(self) -> object:
        head = self.read_head()
        self.read_sessions()
        tail = self.buffer[self.position :] + self.source.read()
        return orjson.loads(head + '"sessions":[]' + tail)

    def read_head(self) -> str:
        while (found := SESSIONS_KEY.search(self.buffer)) is None:
            if not self.more():
                raise BadInput("sessions object has no 'sessions' array")
        self.position = found.end()
        return self.buffer[: found.start()]

    def read_sessions(self) -> None:
        while True:
            self.skip_separators()
            if self.position >= len(self.buffer):
                if not self.more():
                    raise BadInput("sessions array is not closed")
                continue
            if self.buffer[self.position] == "]":
                self.position += 1
                return
            self.sessions.add(checked_session(self.next_value()))

    def next_value(self) -> object:
        while True:
            try:
                value, self.position = self.decoder.raw_decode(self.buffer, self.position)
                return value
            except json.JSONDecodeError:
                if not self.more():
                    raise

    def skip_separators(self) -> None:
        while self.position < len(self.buffer) and self.buffer[self.position] in SEPARATORS:
            self.position += 1

    def more(self) -> bool:
        chunk = self.source.read(CHUNK_CHARS)
        if not chunk:
            return False
        self.buffer = self.buffer[self.position :] + chunk
        self.position = 0
        return True


def checked_session(value: object) -> list[int]:
    if not isinstance(value, list):
        raise BadInput("every session must be an array of track ids")
    for item in value:
        if isinstance(item, bool) or not isinstance(item, int) or not 0 <= item <= MAX_TRACK_ID:
            raise BadInput(f"track id {item!r} is not an unsigned 64-bit integer")
    return value


def hit_rates(
    keys: list[str],
    vectors: NDArray[np.float32],
    sessions: Sessions,
    corpus: Split,
    *,
    window: int,
) -> tuple[float, float]:
    index = {key: row for row, key in enumerate(keys)}
    cases = evaluation_cases(Split(sessions, held_out=True), index, window)
    if not cases:
        return 0.0, 0.0
    popular = popularity_order(corpus, index)
    model_hits = 0
    for start in range(0, len(cases), EVAL_BATCH):
        batch = cases[start : start + EVAL_BATCH]
        model_hits += model_top_hits(batch, vectors)
    popularity_hits = sum(target in top_excluding(popular, context) for context, target in cases)
    return model_hits / len(cases), popularity_hits / len(cases)


def evaluation_cases(
    held_out: Split, index: Mapping[str, int], window: int
) -> list[tuple[list[int], int]]:
    cases: list[tuple[list[int], int]] = []
    for session in held_out:
        for position in range(1, len(session)):
            target = index.get(session[position])
            context = [
                index[item]
                for item in session[max(0, position - window) : position]
                if item in index
            ]
            if target is None or not context:
                continue
            cases.append((context, target))
            if len(cases) >= MAX_EVAL_CASES:
                return cases
    return cases


def model_top_hits(batch: list[tuple[list[int], int]], vectors: NDArray[np.float32]) -> int:
    queries = unit_rows(np.stack([vectors[context].mean(axis=0) for context, _ in batch]))
    scores = queries @ vectors.T
    for row, (context, _) in enumerate(batch):
        scores[row, context] = -np.inf
    top = min(TOP_K, scores.shape[1])
    best = np.argpartition(-scores, top - 1, axis=1)[:, :top]
    return sum(int(target in best[row]) for row, (_, target) in enumerate(batch))


def popularity_order(corpus: Split, index: Mapping[str, int]) -> list[int]:
    counts = Counter(index[item] for session in corpus for item in session if item in index)
    return [row for row, _ in counts.most_common()]


def top_excluding(popular: list[int], context: list[int]) -> list[int]:
    excluded = set(context)
    return [row for row in popular[: TOP_K + len(excluded)] if row not in excluded][:TOP_K]


def unit_rows(rows: NDArray[np.float32]) -> NDArray[np.float32]:
    norms = np.linalg.norm(rows, axis=1, keepdims=True)
    return (rows / np.where(norms == 0.0, 1.0, norms)).astype(np.float32)


def write_vectors(
    path: Path, keys: list[str], vectors: NDArray[np.float32], training: Training
) -> None:
    metrics = {
        "hr_at_20": training.hr_at_20,
        "popularity_hr_at_20": training.popularity_hr_at_20,
        "sessions": training.sessions,
        "vocab": training.vocab,
    }
    with path.open("wb") as sink:
        sink.write(b'{"dim":%d,"points":[' % DIM)
        for row, key in enumerate(keys):
            if row:
                sink.write(b",")
            point = {"id": int(key), "vec": vectors[row]}
            sink.write(orjson.dumps(point, option=orjson.OPT_SERIALIZE_NUMPY))
        sink.write(b'],"metrics":' + orjson.dumps(metrics) + b"}")


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
