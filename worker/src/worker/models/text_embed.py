from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import TypeGuard

import numpy as np
import torch
from sentence_transformers import SentenceTransformer

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

MAX_TOKENS = 8192
EMBED = "embed"
PROMPTS: Mapping[str, str | None] = {"document": None, "query": "query"}
WARMUP_TEXTS = ("warm up the lyrics encoder", "прогрев модели текстов")


class TextEmbedSlot:
    def __init__(self) -> None:
        self._model: SentenceTransformer | None = None

    def load(self, spec: SlotSpec) -> None:
        device = torch.device(spec.device)
        dtype = torch.float16 if device.type == "cuda" else torch.float32
        model = SentenceTransformer(
            spec.model,
            revision=spec.revision,
            device=str(device),
            model_kwargs={"dtype": dtype},
        )
        model.max_seq_length = MAX_TOKENS
        model.requires_grad_(False)
        self._model = model.eval()

    def warmup(self) -> None:
        for kind in PROMPTS:
            self.invoke(EMBED, {}, {"texts": list(WARMUP_TEXTS), "kind": kind})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != EMBED:
            raise BadInput(f"text has no method {method!r}")
        if self._model is None:
            raise RuntimeError("text slot is not loaded")
        kind = args.get("kind")
        if not isinstance(kind, str) or kind not in PROMPTS:
            raise BadInput(f"kind must be one of {sorted(PROMPTS)}")
        texts = args.get("texts")
        if not is_text_list(texts):
            raise BadInput("texts must be a non-empty list of strings")
        prompt_name = PROMPTS[kind]
        for index, tokens in enumerate(token_counts(self._model, texts, prompt_name)):
            if tokens > MAX_TOKENS:
                raise BadInput(f"text {index} has {tokens} tokens, limit {MAX_TOKENS}")
        vectors = self._model.encode(
            list(texts),
            prompt_name=prompt_name,
            batch_size=len(texts),
            normalize_embeddings=True,
            convert_to_numpy=True,
        )
        return {"vectors": np.ascontiguousarray(vectors, dtype=np.float32)}, {}

    def unload(self) -> None:
        self._model = None


def token_counts(
    model: SentenceTransformer, texts: Sequence[str], prompt_name: str | None
) -> list[int]:
    prompt = model.prompts.get(prompt_name or "") or ""
    encoded = model.tokenizer([prompt + text for text in texts])["input_ids"]
    return [len(ids) for ids in encoded]


def is_text_list(value: object) -> TypeGuard[Sequence[str]]:
    return (
        isinstance(value, Sequence)
        and not isinstance(value, str)
        and len(value) > 0
        and all(isinstance(item, str) for item in value)
    )
