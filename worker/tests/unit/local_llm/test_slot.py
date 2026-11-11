from __future__ import annotations

import pytest

from worker.models.local_llm.slot import LocalLlmSlot, Options
from worker.settings import LOCAL_LLM_METHOD

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


def test_the_marks_of_a_call_reach_the_prompt_with_its_system_text(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    slot = LocalLlmSlot()
    answered: list[tuple[object, ...]] = []
    monkeypatch.setattr(slot, "_answer", lambda *asked: answered.append(asked) or {})
    call = {"system": "Rules. Persona.", "messages": [], "schema": {}, "max_tokens": 4}

    slot.invoke(LOCAL_LLM_METHOD, {}, {**call, "system_marks": [6]})
    with pytest.raises(TypeError):
        slot.invoke(LOCAL_LLM_METHOD, {}, call)

    assert [asked[:2] for asked in answered] == [("Rules. Persona.", [6])]
