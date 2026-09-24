from __future__ import annotations

import logging
from collections.abc import Mapping

from worker.domain import embedding, language
from worker.domain.deadline import Deadline
from worker.domain.outcome import Outcome, Reason, TransientFailure
from worker.domain.ports import Engines, EngineUnavailable
from worker.observability.counters import Counters

LYRICS_DIM = 1024
LANE = "lyrics"

log = logging.getLogger(__name__)


class EmbedLyricsLane:
    def __init__(self, engines: Engines, counters: Counters) -> None:
        self._engines = engines
        self._counters = counters

    async def process(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        try:
            return await self._embed(request, deadline)
        except Exception as error:
            return embedding.outcome_of_error(error, LANE, self._counters)

    async def _embed(self, request: Mapping[str, object], deadline: Deadline) -> Outcome:
        text = embedding.text_field(request, "text")
        hint = language.to_wire(embedding.optional_text_field(request, "language"))
        if not text.strip():
            return Outcome.of(Reason.EMPTY_TEXT, language=hint)
        detected = hint if hint is not None else await self._detect(text, deadline)
        vectors = await self._engines.embed_text([text], "document", deadline)
        vec = embedding.single(vectors, LYRICS_DIM, "text", self._counters)
        return Outcome.ok(vec=vec, language=detected)

    async def _detect(self, text: str, deadline: Deadline) -> str | None:
        try:
            detection = await language.detect(text, None, self._engines, deadline)
        except (EngineUnavailable, TransientFailure) as error:
            if isinstance(error, TransientFailure) and error.reason is Reason.DEADLINE_EXCEEDED:
                raise
            self._counters.inc("language_detect_failures_total", lane=LANE)
            log.warning("language detection failed, using the script", extra={"error": repr(error)})
            lines = " ".join(language.lines_for_detection(text))
            return language.to_wire(language.script_fallback(lines, {}))
        return language.to_wire(detection.track)
