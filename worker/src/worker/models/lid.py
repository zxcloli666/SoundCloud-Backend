from __future__ import annotations

import hashlib
import importlib
import os
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from worker.fetch_models import FASTTEXT_HOME_ENV
from worker.runtime.protocol import Arrays, BadInput, SlotSpec

DETECT = "detect_language"
TOP_K = 5
LABEL_PREFIX = "__label__"
HASH_CHUNK_BYTES = 1 << 20
WARMUP_LINES = ("warming up the language model", "прогреваем модель языка")


class LidSlot:
    def __init__(self) -> None:
        self._model: Any = None

    def load(self, spec: SlotSpec) -> None:
        path = weights_path(spec.model, os.environ)
        digest = sha256_of(path)
        if digest != spec.revision:
            raise RuntimeError(f"{path} sha256 {digest} does not match {spec.revision}")
        fasttext = importlib.import_module("fasttext")
        self._model = fasttext.load_model(str(path))

    def warmup(self) -> None:
        self.invoke(DETECT, {}, {"lines": list(WARMUP_LINES)})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != DETECT:
            raise BadInput(f"lid has no method {method!r}")
        lines = checked_lines(args.get("lines"))
        if not lines:
            return {}, {"guesses": []}
        labels, probs = self._model.predict(lines, k=TOP_K)
        guesses = [
            [
                (label.removeprefix(LABEL_PREFIX), min(float(prob), 1.0))
                for label, prob in zip(row, scores, strict=True)
            ]
            for row, scores in zip(labels, probs, strict=True)
        ]
        return {}, {"guesses": guesses}

    def unload(self) -> None:
        self._model = None


def weights_path(url: str, environ: Mapping[str, str]) -> Path:
    home = environ.get(FASTTEXT_HOME_ENV, "")
    if not home:
        raise RuntimeError(f"{FASTTEXT_HOME_ENV} is not set")
    return Path(home) / url.rsplit("/", 1)[-1]


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(HASH_CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest()


def checked_lines(lines: object) -> list[str]:
    if not isinstance(lines, Sequence) or isinstance(lines, str):
        raise BadInput("lines must be a list of strings")
    checked: list[str] = []
    for line in lines:
        if not isinstance(line, str) or "\n" in line:
            raise BadInput("each line must be a string without line breaks")
        checked.append(line)
    return checked
