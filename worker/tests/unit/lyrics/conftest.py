from __future__ import annotations

import io
from collections.abc import AsyncIterator, Sequence
from dataclasses import dataclass
from pathlib import Path

import aiohttp
import numpy as np
import pytest
import soundfile as sf

from tests.fakes.engines import FakeEngines
from tests.fakes.http_audio import FakeAudioServer
from worker.domain.audio.source import AudioSource
from worker.domain.lyrics import text, tokens
from worker.domain.lyrics.placement import LineTiming, Source, Word
from worker.domain.lyrics.tokens import TokenizedLine
from worker.domain.lyrics.transcribe import TranscribeLane
from worker.domain.ports import Alignment, Draft, LanguageGuess, Span, TokenSpan
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

SYNC_VERSION = "s4.c07281df.49402e95.deadbeef"
TRACK_S = 40.0
REGION_S = 6.0
REGIONS = [Span(2.0, 8.0), Span(9.0, 15.0), Span(16.0, 22.0), Span(23.0, 29.0), Span(30.0, 36.0)]
RUSSIAN_LINES = [
    "Зашей мне глаза чтоб я не видел тебя",
    "Тело гниёт зарастая в цветах",
    "Просто забудь меня просто забудь меня",
    "Мы сияем как в последний раз здесь",
    "Тебе с нами нельзя нет нет",
]


def lines_of(texts: Sequence[str], language: str | None = "ru") -> list[TokenizedLine]:
    script = text.parse("\n".join(texts))
    return tokens.tokenize_script(script, [language] * len(script.entries))


def timing(
    line: int,
    start_s: float,
    end_s: float,
    *,
    words: int = 3,
    source: Source = "qwen",
    region: int | None = 0,
    score: float = 0.9,
    similarity: float = 0.9,
    word_s: float = 0.3,
) -> LineTiming:
    step = (end_s - start_s) / max(1, words)
    placed = tuple(
        Word(
            line,
            f"w{index}",
            round(start_s + index * step, 3),
            round(start_s + index * step + min(word_s, step), 3),
            score,
        )
        for index in range(words)
    )
    return LineTiming(
        line=line,
        start_s=start_s,
        end_s=end_s,
        source=source,
        words=placed,
        region=region,
        region_score=score,
        anchor_similarity=similarity,
    )


def wav_bytes(seconds: float = TRACK_S, sample_rate: int = 44_100) -> bytes:
    t = np.arange(int(sample_rate * seconds)) / sample_rate
    tone = 0.2 * np.sin(2 * np.pi * 220 * t) * (0.5 + 0.5 * np.sin(2 * np.pi * 0.5 * t))
    stereo = np.stack([tone, tone], axis=1).astype(np.float32)
    buffer = io.BytesIO()
    sf.write(buffer, stereo, sample_rate, format="WAV", subtype="PCM_16")
    return buffer.getvalue()


@dataclass
class LaneHarness:
    lane: TranscribeLane
    engines: FakeEngines
    counters: Counters
    server: FakeAudioServer

    def request(self, reference_text: str, /, **overrides: object) -> dict[str, object]:
        request: dict[str, object] = {
            "sc_track_id": "98765",
            "upload_generation": 3,
            "attempt": 1,
            "audio_url": self.server.url("/ok.wav"),
            "reference_text": reference_text,
            "reference_lines_total": len(
                [line for line in reference_text.splitlines() if line.strip()]
            ),
            "language": "ru",
            "mode": "align",
        }
        request.update(overrides)
        return request


def inside_region_alignment(**kwargs: object) -> Alignment:
    count = len(list(kwargs["tokens"]))
    step = REGION_S / max(1, count)
    spans = tuple(
        TokenSpan(round(index * step, 3), round((index + 1) * step - 0.01, 3), 0.9)
        for index in range(count)
    )
    return Alignment(spans, 0.9)


@pytest.fixture
def russian_engines(engines: FakeEngines) -> FakeEngines:
    engines.regions = list(REGIONS)
    engines.default_language = [LanguageGuess("ru", 0.97)]
    engines.drafts = [Draft(line.lower(), "ru", 0.95) for line in RUSSIAN_LINES]
    engines.overrides["align"] = inside_region_alignment
    return engines


@pytest.fixture
async def harness(
    russian_engines: FakeEngines, settings: Settings, work_dir: Path
) -> AsyncIterator[LaneHarness]:
    counters = Counters()
    async with FakeAudioServer(body=wav_bytes()) as server, aiohttp.ClientSession() as session:
        source = AudioSource(session, timeout_s=30, max_bytes=96 << 20, counters=counters)
        lane = TranscribeLane(
            russian_engines,
            source,
            Workspace(work_dir, counters),
            settings.audio,
            settings.sync,
            SYNC_VERSION,
            counters,
        )
        yield LaneHarness(lane, russian_engines, counters, server)
