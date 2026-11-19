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
            "hints": {"chorus_ms": [61000, 121000], "comment_ms": [62000], "replay_ms": []},
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
        "highlight": {
            "sc_track_id": "98765",
            "s3_url": "https://storage.example/audio/98765.opus",
            "attempt": 2,
            "version": 2,
            "hints": {"chorus_ms": [61000], "comment_ms": [], "replay_ms": [60500]},
        },
        "video-find": {
            "sc_track_id": "98765",
            "attempt": 2,
            "s3_url": "https://storage.example/audio/98765.opus",
            "artist": "Central Cee",
            "title": "Doja",
            "duration_ms": 96000,
            "candidates": [
                {
                    "index": 0,
                    "source": "youtube",
                    "kind": "video",
                    "url": "https://storage.example/video-tmp/98765-2-0.mp4",
                    "source_ref": "_VuJA-VQRcY",
                },
                {
                    "index": 1,
                    "source": "spotify",
                    "kind": "canvas",
                    "url": "https://storage.example/video-tmp/98765-2-1.mp4",
                    "source_ref": "spotify:track:1",
                },
            ],
        },
        "video-vlm": {
            "sc_track_id": "98765",
            "attempt": 2,
            "video_url": "https://storage.example/video-tmp/98765-2-0.mp4",
            "duration_ms": 96000,
            "offset_ms": 1500,
        },
        "video-keyframe": {
            "sc_track_id": "98765",
            "attempt": 2,
            "step": 3,
            "cover_url": "https://i1.sndcdn.com/artworks-000-t500x500.jpg",
            "prompt": "the same girl from the cover walks through rain at night",
            "negative_prompt": "text, logo, extra people",
            "width": 720,
            "height": 720,
            "seed": 11,
        },
        "video-shot": {
            "sc_track_id": "98765",
            "attempt": 2,
            "step": 4,
            "frame_url": "https://storage.example/video-tmp/gen/98765-2-3.png",
            "prompt": "slow push in, hair moves in the wind",
            "negative_prompt": "",
            "width": 720,
            "height": 720,
            "frames": 81,
            "fps": 16,
            "stills": 4,
            "seed": 12,
        },
        "video-assemble": {
            "sc_track_id": "98765",
            "attempt": 2,
            "step": 9,
            "audio_url": "https://storage.example/audio/98765.opus",
            "audio_offset_ms": 52500,
            "shots": [
                {
                    "url": "https://storage.example/video-tmp/gen/98765-2-4.mp4",
                    "trim_start_ms": 0,
                    "duration_ms": 5000,
                }
            ],
            "width": 720,
            "height": 720,
            "fps": 16,
        },
    }
)

RPC_REQUESTS: Mapping[str, Mapping[str, object]] = MappingProxyType(
    {
        "ai.rpc.complete": {
            "system": "You are the voice of a music player. Answer with one JSON object.",
            "messages": [
                {"role": "user", "content": "сделай потише"},
                {"role": "assistant", "content": '{"say": "Готово", "actions": []}'},
                {"role": "user", "content": "and what is playing now?"},
            ],
            "schema": {
                "type": "object",
                "properties": {"say": {"type": "string"}, "actions": {"type": "array"}},
                "required": ["say", "actions"],
                "additionalProperties": False,
            },
            "max_tokens": 512,
        },
        "speak.rpc.say": {"voice": "loli", "text": "Включаю Kendrick Lamar, Luther."},
    }
)

RPC_REPLIES: Mapping[str, tuple[Mapping[str, object], ...]] = MappingProxyType(
    {
        "ai.rpc.complete.reply": (
            {
                "ok": True,
                "data": {
                    "text": '{"say": "Kendrick Lamar, Luther", "actions": []}',
                    "model": "org/model-GGUF/model-Q4_K_M.gguf@0123abcd",
                    "input_tokens": 1500,
                    "output_tokens": 80,
                },
            },
        ),
        "speak.rpc.say.reply": (
            {"ok": True, "data": {"seconds": 2.48, "clip": "T2dnUwACAAAAAAAAAAA="}},
        ),
    }
)
RPC_ERRORS = ("expired", "invalid_request", "internal")

GEN_REASONS = frozenset(
    {
        Reason.INVALID_REQUEST,
        Reason.MODEL_OUTPUT_INVALID,
        Reason.DEADLINE_EXCEEDED,
        Reason.ENGINE_CRASHED,
        Reason.OUT_OF_MEMORY,
        Reason.DOWNLOAD_FAILED,
        Reason.INTERNAL_ERROR,
        Reason.ENGINE_RESTARTED,
        Reason.INPUTS_GONE,
    }
)
GEN_FILE = {
    "bytes": 3,
    "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
}

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
        "video-find": frozenset(
            {
                Reason.AUDIO_NOT_FOUND,
                Reason.AUDIO_FORBIDDEN,
                Reason.SILENT_AUDIO,
                Reason.INVALID_REQUEST,
                Reason.UNDECODABLE_AUDIO,
                Reason.AUDIO_TOO_LONG,
                Reason.AUDIO_TOO_LARGE,
                Reason.DOWNLOAD_FAILED,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
                Reason.NO_VIDEO_FOUND,
                Reason.STATIC_VIDEO,
                Reason.INPUTS_GONE,
            }
        ),
        "video-vlm": frozenset(
            {
                Reason.INVALID_REQUEST,
                Reason.UNDECODABLE_AUDIO,
                Reason.AUDIO_TOO_LARGE,
                Reason.MODEL_OUTPUT_INVALID,
                Reason.DEADLINE_EXCEEDED,
                Reason.ENGINE_CRASHED,
                Reason.OUT_OF_MEMORY,
                Reason.DOWNLOAD_FAILED,
                Reason.INTERNAL_ERROR,
                Reason.ENGINE_RESTARTED,
                Reason.INPUTS_GONE,
            }
        ),
        "video-keyframe": GEN_REASONS,
        "video-shot": GEN_REASONS,
        "video-assemble": GEN_REASONS | {Reason.UNDECODABLE_AUDIO},
        "highlight": frozenset(
            {
                Reason.AUDIO_NOT_FOUND,
                Reason.AUDIO_FORBIDDEN,
                Reason.SILENT_AUDIO,
                Reason.INVALID_REQUEST,
                Reason.UNDECODABLE_AUDIO,
                Reason.AUDIO_TOO_LONG,
                Reason.AUDIO_TOO_LARGE,
                Reason.DOWNLOAD_FAILED,
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
                return {
                    "mert": vector(1024),
                    "clap": vector(512),
                    "fingerprint": "AQAD" * 16,
                    "moment": {"version": 2, "start_ms": 52500, "length_ms": 30000, "score": 0.83},
                }
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
        case "highlight":
            if ok:
                return {"version": 2, "start_ms": 52500, "length_ms": 30000, "score": 0.83}
            return {}
        case "video-find":
            if ok:
                return {
                    "kind": "loop",
                    "uncertain": False,
                    "candidate": 1,
                    "offset_ms": 0,
                    "duration_ms": 6000,
                    "loop": True,
                    "scores": {"ber": 0.0211, "peak": 428.0},
                }
            return {}
        case "video-vlm":
            if ok:
                return {
                    "label": "loop",
                    "model": "Qwen/Qwen3-VL-2B-Instruct-GGUF/Qwen3VL-2B-Q4_K_M.gguf@52d6c8ff",
                    "scores": {"clip": 0.1, "loop": 0.8, "static": 0.1},
                }
            return {}
        case "video-keyframe" | "video-shot" | "video-assemble":
            if ok:
                return {
                    "clip": {**GEN_FILE, "parts": 1},
                    "files": [{"name": "frame.png", "offset": 0, "bytes": 3}],
                    "part": {"index": 0, "data": "YWJj"},
                }
            return {}
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
