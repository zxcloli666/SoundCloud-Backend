from __future__ import annotations

import pytest

from worker.models.local_llm.slot import LocalLlmSlot, Options

CHUNK = 8


class Words:
    def tokenize(self, text: str, *, special: bool) -> list[int]:
        return list(range(len(text.split())))


def test_a_model_is_warmed_up_only_while_its_read_pace_is_unknown(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    slot = LocalLlmSlot()
    warmups: list[tuple[object, ...]] = []
    monkeypatch.setattr(slot, "_model", Words())
    monkeypatch.setattr(slot, "_source", ("model.gguf", Options("model.gguf", 4096, CHUNK, 1)))
    monkeypatch.setattr(slot, "_answer", lambda *asked: warmups.append(asked))

    slot.warmup()
    slot._reading.learn(CHUNK, 1.0)
    slot.warmup()

    assert len(warmups) == 1
