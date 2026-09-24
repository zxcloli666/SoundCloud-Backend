from __future__ import annotations

import hashlib
from collections.abc import Mapping

from worker.domain import embedding
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, PermanentFailure, Reason
from worker.domain.ports import Engines, Float32Array
from worker.observability.counters import Counters

MULAN_TEXT_DIM = 512
LYRICS_DIM = 1024
LANE = "encode"
MAX_TEXT_BYTES = 512


class EncodeTextLane:
    def __init__(self, engines: Engines, counters: Counters) -> None:
        self._engines = engines
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._encode(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters, vector=None)

    async def _encode(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        model = embedding.text_field(request, "model")
        text = embedding.text_field(request, "text")
        digest = embedding.text_field(request, "hash")
        encoded = text.encode("utf-8")
        if len(encoded) > MAX_TEXT_BYTES:
            raise PermanentFailure(
                Reason.INVALID_REQUEST, f"text is {len(encoded)} bytes, limit {MAX_TEXT_BYTES}"
            )
        if hashlib.sha256(encoded).hexdigest() != digest:
            raise PermanentFailure(Reason.HASH_MISMATCH, f"hash={digest[:16]}")
        if not text.strip():
            return Outcome.of(Reason.EMPTY_TEXT, vector=None)
        vector = await self._vector(model, text, deadline)
        return Outcome.ok(vector=vector)

    async def _vector(self, model: str, text: str, deadline: Deadline) -> Float32Array:
        if model == "mulan":
            rows = await self._engines.embed_text_mulan([text], deadline)
            return embedding.single(rows, MULAN_TEXT_DIM, "mulan", self._counters)
        if model == "lyrics":
            rows = await self._engines.embed_text([text], "query", deadline)
            return embedding.single(rows, LYRICS_DIM, "text", self._counters)
        raise PermanentFailure(Reason.INVALID_REQUEST, f"model={model}")
