from __future__ import annotations

import hashlib
from collections.abc import Mapping
from types import MappingProxyType

import numpy as np

from worker.contract import LaneSpec
from worker.domain.outcome import Outcome, Producer, Reason, Status, status_of

WORST_FLOAT = -0.012345679
ENCODE_TEXT = "ночной город"
SYNC_VERSION = "s4.c07281df.49402e95.9a8b7c6d"

REQUESTS: Mapping[str, Mapping[str, object]] = MappingProxyType(
    {
        "audio": {
            "sc_track_id": "98765",
            "s3_url": "https://storage.example/audio/98765.opus",
            "upload_generation": 3,
            "attempt": 1,
        },
        "lyrics": {
            "sc_track_id": "98765",
            "request_id": "lyr:98765:4",
            "text": "Я иду по городу\nИ не знаю куда",
            "language": None,
        },
        "transcribe": {
            "sc_track_id": "98765",
            "upload_generation": 3,
            "attempt": 2,
            "audio_url": "https://storage.example/audio/98765.opus",
            "reference_text": "Я иду по городу\nИ не знаю куда",
            "reference_lines_total": 2,
            "language": "ru",
            "mode": "align",
        },
        "encode": {
            "model": "lyrics",
            "text": ENCODE_TEXT,
            "hash": hashlib.sha256(ENCODE_TEXT.encode()).hexdigest(),
        },
        "collab": {
            "object": "collab-input-7c1d",
            "dataset_version": 2,
            "dim": 128,
            "min_count": 5,
            "window": 5,
            "epochs": 10,
            "negative": 10,
        },
        "taste": {
            "object": "taste-input-9f2e",
            "dataset_version": 1,
            "dim": 128,
            "epochs": 20,
            "batch_size": 1024,
            "negatives": 50,
            "seed": 7,
            "previous_version": None,
        },
    }
)

RPC_REQUESTS: Mapping[str, Mapping[str, object]] = MappingProxyType(
    {
        "ai.rpc.resolve_artist": {
            "title": "Kendrick Lamar - Luther (feat. SZA)",
            "uploader": "kendricklamar",
            "metadata_artist": None,
            "isrc": None,
            "description": None,
            "duration_ms": 177000,
        },
        "ai.rpc.match_track": {
            "target": {"artist": "Kendrick Lamar", "title": "Luther"},
            "candidates": [
                {"id": 1, "artist": "Kendrick Lamar", "title": "luther", "duration_sec": 177.0},
                {"id": 4294967295, "artist": "SZA", "title": "Saturn"},
            ],
        },
    }
)

RPC_REPLIES: Mapping[str, tuple[Mapping[str, object], ...]] = MappingProxyType(
    {
        "ai.rpc.resolve_artist.reply": (
            {
                "ok": True,
                "data": {
                    "primary_artist": "Kendrick Lamar",
                    "featured": ["SZA"],
                    "producers": [],
                    "remixers": [],
                    "album": None,
                    "confidence": 0.93,
                    "source": "llm",
                },
            },
            {
                "ok": True,
                "data": {
                    "primary_artist": None,
                    "featured": [],
                    "producers": [],
                    "remixers": [],
                    "album": {"title": "GNX", "year": 2024},
                    "confidence": 0.0,
                    "source": "deterministic",
                },
            },
        ),
        "ai.rpc.match_track.reply": (
            {"ok": True, "data": {"match_id": 1, "confidence": 0.88, "source": "deterministic"}},
            {"ok": True, "data": {"match_id": None, "confidence": 0.41, "source": "llm"}},
        ),
    }
)
RPC_ERRORS = ("expired", "invalid_request", "internal")

DESIGN_REASONS: Mapping[str, frozenset[Reason]] = MappingProxyType(
    {
        "audio": frozenset(
            {
                Reason.AUDIO_NOT_FOUND,
                Reason.AUDIO_FORBIDDEN,
                Reason.SILENT_AUDIO,
                Reason.AUDIO_TOO_SHORT,
                Reason.INVALID_REQUEST,
                Reason.UNDECODABLE_AUDIO,
                Reason.AUDIO_TOO_LONG,
                Reason.AUDIO_TOO_LARGE,
                Reason.MODEL_OUTPUT_INVALID,
                Reason.DOWNLOAD_FAILED,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
            }
        ),
        "lyrics": frozenset(
            {
                Reason.EMPTY_TEXT,
                Reason.INVALID_REQUEST,
                Reason.TEXT_TOO_LONG_FOR_MODEL,
                Reason.MODEL_OUTPUT_INVALID,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
            }
        ),
        "transcribe": frozenset(
            {
                Reason.LOW_CONFIDENCE,
                Reason.TOO_FEW_LINES_PLACED,
                Reason.NO_VOCAL_DETECTED,
                Reason.UNSUPPORTED_LANGUAGE,
                Reason.LYRICS_MISMATCH,
                Reason.OUT_OF_ORDER,
                Reason.PLACED_IN_SILENCE,
                Reason.AUDIO_NOT_FOUND,
                Reason.AUDIO_FORBIDDEN,
                Reason.EMPTY_REFERENCE_TEXT,
                Reason.SILENT_AUDIO,
                Reason.INVALID_REQUEST,
                Reason.UNDECODABLE_AUDIO,
                Reason.AUDIO_TOO_LONG,
                Reason.AUDIO_TOO_LARGE,
                Reason.MODEL_OUTPUT_INVALID,
                Reason.DOWNLOAD_FAILED,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
            }
        ),
        "encode": frozenset(
            {
                Reason.EMPTY_TEXT,
                Reason.INVALID_REQUEST,
                Reason.HASH_MISMATCH,
                Reason.TEXT_TOO_LONG_FOR_MODEL,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
            }
        ),
        "collab": frozenset(
            {
                Reason.EMPTY_VOCAB,
                Reason.OBJECT_NOT_FOUND,
                Reason.BELOW_BASELINE,
                Reason.INVALID_REQUEST,
                Reason.OBJECT_STORE_UNAVAILABLE,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
            }
        ),
        "taste": frozenset(
            {
                Reason.TOO_FEW_USERS,
                Reason.OBJECT_NOT_FOUND,
                Reason.BELOW_BASELINE,
                Reason.INVALID_REQUEST,
                Reason.OBJECT_STORE_UNAVAILABLE,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
            }
        ),
    }
)


def producer(sync_version: str | None = None) -> Producer:
    return Producer(
        worker_id="gpu-main",
        build="2026.09.1-cu126",
        models={"mert": "OpenMuQ/MuQ-large-msd-iter@0562a578"},
        sync_version=sync_version,
    )


def vector(dim: int) -> list[float]:
    return float32_vector(dim).tolist()


def float32_vector(dim: int) -> np.ndarray:
    return np.full(dim, WORST_FLOAT, dtype=np.float32)


def outcome_for(lane: str, reason: Reason | None) -> Outcome:
    status = Status.OK if reason is None else status_of(reason)
    fields = fields_for(lane, status)
    if reason is None:
        return Outcome.ok(**fields)
    return Outcome.of(reason, "stage=decode", **fields)


def fields_for(lane: str, status: Status) -> dict[str, object]:
    ok = status is Status.OK
    match lane:
        case "audio":
            if ok:
                return {"mert": vector(1024), "clap": vector(512), "fingerprint": "AQAD" * 16}
            return {}
        case "lyrics":
            if ok:
                return {"vec": vector(1024), "language": "ru"}
            return {"language": None} if status is Status.EMPTY else {}
        case "transcribe":
            return transcribe_fields(status)
        case "encode":
            return {"vector": vector(1024) if ok else None}
        case "collab":
            if ok:
                return {
                    "trained": True,
                    "object": "collab-input-7c1d-vectors",
                    "dim": 128,
                    "points_count": 18342,
                }
            return {"trained": False, "dim": 128, "points_count": 0}
        case "taste":
            return taste_fields() if ok else {"dim": 128}
    raise AssertionError(f"no fields for lane {lane}")


def transcribe_fields(status: Status) -> dict[str, object]:
    fields: dict[str, object] = {"sync_version": SYNC_VERSION}
    if status in (Status.OK, Status.REJECTED):
        fields |= {
            "confidence": 0.31,
            "placed_share": 0.55,
            "aligned_share": 0.48,
            "lines_total": 2,
            "lines_unplaced": 1,
            "language": "ru",
        }
    if status is Status.OK:
        fields |= {
            "confidence": 0.91,
            "placed_share": 1.0,
            "aligned_share": 1.0,
            "lines_unplaced": 0,
            "synced_lrc": "[00:12.30]Я иду по городу\n[00:15.10]И не знаю куда",
            "words": [
                {
                    "line": 0,
                    "text": "Я",
                    "start_ms": 12300,
                    "end_ms": 12500,
                    "interpolated": False,
                    "unplaced": False,
                }
            ],
        }
    return fields


def taste_fields() -> dict[str, object]:
    return {
        "version": "taste-202609230400-1a2b3c4d",
        "object": "taste-202609230400-1a2b3c4d",
        "dim": 128,
        "items_count": 52000,
        "users_count": 1800,
        "metrics": {
            "recall_at_50": 0.21,
            "ndcg_at_20": 0.09,
            "cold_recall_at_50": 0.11,
            "coverage_at_50": 0.4,
            "baselines": {"popularity": 0.08, "item2vec": 0.17, "content": 0.12},
        },
    }


def done_payload(lane: LaneSpec, reason: Reason | None) -> dict[str, object]:
    sync_version = SYNC_VERSION if lane.name == "transcribe" else None
    echo = lane.echo_fields(REQUESTS[lane.name])
    return outcome_for(lane.name, reason).to_done(echo, producer(sync_version))
