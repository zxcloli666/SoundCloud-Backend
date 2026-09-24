from __future__ import annotations

import json
import os
from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

import pytest

from tests.conftest import BASE_ENV, CONFIG_DIR, WORKER_ROOT
from worker import settings as settings_module
from worker.domain import language
from worker.domain.deadline import Deadline
from worker.domain.ports import LanguageGuess
from worker.fetch_models import FASTTEXT_HOME_ENV
from worker.runtime.protocol import BadInput, SlotSpec

pytestmark = pytest.mark.models

MANIFEST = WORKER_ROOT / "eval" / "manifest.json"
MIN_TEXT_CHARS = 200
LINE_CHARS = range(20, 61)
MIN_TEXT_ACCURACY = 0.97
MIN_LINE_ACCURACY = 0.9
MIN_TEXTS = 30
NATIVE_SCRIPT = {"hi": "Devanagari", "sr": "Cyrillic"}


@dataclass(frozen=True)
class LabelledText:
    track: str
    language: str
    text: str


def lid_spec() -> SlotSpec:
    config = settings_module.load(CONFIG_DIR, BASE_ENV).slots["cpu-tools"]
    return SlotSpec(
        "cpu-tools", "worker.models.lid:LidSlot", config.model, config.revision, "cpu", 1, 0
    )


def labelled_texts() -> list[LabelledText]:
    data = os.environ.get("EVAL_DATA_DIR")
    if not data:
        pytest.skip("needs the eval texts (EVAL_DATA_DIR)")
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    texts: dict[str, LabelledText] = {}
    for track in manifest["tracks"]:
        if track["kind"] != "positive" or track["input_text"] in texts:
            continue
        text = (Path(data) / track["input_text"]).read_text(encoding="utf-8")
        if in_native_script(text, track["language"]):
            texts[track["input_text"]] = LabelledText(track["id"], track["language"], text)
    return list(texts.values())


def in_native_script(text: str, code: str) -> bool:
    script = NATIVE_SCRIPT.get(code)
    return script is None or language.dominant_script(text) == script


@pytest.fixture(scope="module")
def lid() -> Iterator[object]:
    if not os.environ.get(FASTTEXT_HOME_ENV):
        pytest.skip(f"needs lid.176.bin in {FASTTEXT_HOME_ENV}")
    from worker.models.lid import LidSlot

    slot = LidSlot()
    slot.load(lid_spec())
    slot.warmup()
    yield slot
    slot.unload()


class LidEngines:
    def __init__(self, slot: object) -> None:
        self._slot = slot

    async def detect_language(
        self, lines: Sequence[str], deadline: Deadline
    ) -> Sequence[Sequence[LanguageGuess]]:
        return [
            [LanguageGuess(code, prob) for code, prob in row] for row in predict(self._slot, lines)
        ]


def predict(slot: object, lines: Sequence[str]) -> list[list[tuple[str, float]]]:
    _, result = slot.invoke("detect_language", {}, {"lines": list(lines)})
    guesses: list[list[tuple[str, float]]] = result["guesses"]
    return guesses


def accuracy(hits: Mapping[str, bool]) -> float:
    return sum(hits.values()) / len(hits)


async def test_track_language_of_long_texts(lid: object) -> None:
    texts = [item for item in labelled_texts() if len(item.text) >= MIN_TEXT_CHARS]
    engines = LidEngines(lid)
    hits = {}
    for item in texts:
        detection = await language.detect(item.text, None, engines, Deadline.after(60))
        hits[item.track] = language.to_wire(detection.track) == item.language

    assert len(texts) >= MIN_TEXTS
    assert accuracy(hits) >= MIN_TEXT_ACCURACY, sorted(k for k, hit in hits.items() if not hit)


def test_lines_of_twenty_to_sixty_characters(lid: object) -> None:
    hits = {}
    for item in labelled_texts():
        lines = [
            line for line in language.lines_for_detection(item.text) if len(line) in LINE_CHARS
        ]
        for index, row in enumerate(predict(lid, lines)):
            hits[f"{item.track}:{index}"] = language.to_wire(row[0][0]) == item.language

    assert accuracy(hits) >= MIN_LINE_ACCURACY


def test_guesses_are_at_most_five_ranked_probabilities(lid: object) -> None:
    rows = predict(lid, ["Я иду по улице ночной", "I walk alone down the empty street"])

    assert [row[0][0] for row in rows] == ["ru", "en"]
    for row in rows:
        assert 1 <= len(row) <= 5
        assert all(0.0 <= prob <= 1.0 for _, prob in row)
        assert [prob for _, prob in row] == sorted((prob for _, prob in row), reverse=True)


def test_no_lines_give_no_guesses(lid: object) -> None:
    assert predict(lid, []) == []


@pytest.mark.parametrize(
    "args", [{}, {"lines": "one line"}, {"lines": ["two\nlines"]}, {"lines": [1]}]
)
def test_bad_lines(lid: object, args: dict[str, object]) -> None:
    with pytest.raises(BadInput):
        lid.invoke("detect_language", {}, args)


def test_unknown_method(lid: object) -> None:
    with pytest.raises(BadInput):
        lid.invoke("detect", {}, {"lines": ["hello there"]})


def test_weights_with_other_hash_are_refused(lid: object) -> None:
    from worker.models.lid import LidSlot

    spec = lid_spec()
    forged = SlotSpec(spec.name, spec.loader, spec.model, "0" * 64, "cpu", 1, 0)

    with pytest.raises(RuntimeError, match="does not match"):
        LidSlot().load(forged)
