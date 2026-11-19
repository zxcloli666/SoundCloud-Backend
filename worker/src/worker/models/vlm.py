from __future__ import annotations

import base64
import math
from collections.abc import Mapping, Sequence
from typing import Any

import cv2
import numpy as np
from huggingface_hub import hf_hub_download
from llama_cpp import Llama
from llama_cpp.llama_chat_format import Qwen25VLChatHandler
from llama_cpp.llama_grammar import LlamaGrammar

from worker.runtime.protocol import CALL_GUARD, Arrays, BadInput, SlotSpec
from worker.settings import VLM_METHOD, VLM_SLOT

JPEG_QUALITY = 90
ANSWER_TOKENS = 4
TOP_LOGPROBS = 20
WARMUP_SIDE = 224


class VlmSlot:
    def __init__(self) -> None:
        self._llm: Llama | None = None

    def load(self, spec: SlotSpec) -> None:
        file = text_option(spec, "file")
        mmproj = text_option(spec, "mmproj")
        model_path = hf_hub_download(
            spec.model, file, revision=spec.revision, local_files_only=True
        )
        mmproj_path = hf_hub_download(
            spec.model, mmproj, revision=spec.revision, local_files_only=True
        )
        self._llm = Llama(
            model_path=model_path,
            chat_handler=Qwen25VLChatHandler(clip_model_path=mmproj_path, verbose=False),
            n_ctx=int_option(spec, "context_tokens"),
            n_threads=CALL_GUARD.threads or None,
            n_threads_batch=CALL_GUARD.threads or None,
            n_gpu_layers=int_option(spec, "gpu_layers"),
            verbose=False,
        )

    def warmup(self) -> None:
        blank = np.full((1, WARMUP_SIDE, WARMUP_SIDE, 3), 128, dtype=np.uint8)
        self.invoke(VLM_METHOD, {"frames": blank}, {"prompt": "Answer: clip", "labels": ["clip"]})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != VLM_METHOD:
            raise BadInput(f"{VLM_SLOT} has no method {method!r}")
        frames, prompt, labels = arrays.get("frames"), args.get("prompt"), args.get("labels")
        if (
            frames is None
            or frames.dtype != np.uint8
            or frames.ndim != 4
            or frames.shape[3] != 3
            or frames.shape[0] < 1
        ):
            raise BadInput("frames must be uint8 [n, height, width, 3]")
        if not isinstance(prompt, str) or not (
            isinstance(labels, list) and labels and all(isinstance(label, str) for label in labels)
        ):
            raise BadInput("prompt must be text and labels a list of words")
        if self._llm is None:
            raise RuntimeError(f"{VLM_SLOT} is not loaded")
        return {}, self._answer(frames, prompt, labels)

    def unload(self) -> None:
        if self._llm is not None:
            self._llm.close()
        self._llm = None

    def _answer(
        self, frames: np.ndarray, prompt: str, labels: Sequence[str]
    ) -> Mapping[str, object]:
        assert self._llm is not None
        content: list[Any] = [
            {"type": "image_url", "image_url": {"url": data_uri(frame)}} for frame in frames
        ]
        content.append({"type": "text", "text": prompt})
        messages: Any = [{"role": "user", "content": content}]
        grammar = LlamaGrammar.from_string(
            "root ::= " + " | ".join(f'"{label}"' for label in labels), verbose=False
        )
        reply: Any = self._llm.create_chat_completion(
            messages=messages,
            temperature=0.0,
            max_tokens=ANSWER_TOKENS,
            grammar=grammar,
            logprobs=True,
            top_logprobs=TOP_LOGPROBS,
        )
        choice = reply["choices"][0]
        label = str(choice["message"]["content"]).strip().lower()
        if label not in labels:
            raise BadInput(f"the model answered {label!r}")
        return {"label": label, "scores": scores_of(choice.get("logprobs"), labels)}


def scores_of(logprobs: Mapping[str, Any] | None, labels: Sequence[str]) -> dict[str, float]:
    content = (logprobs or {}).get("content") or []
    if not content:
        return {}
    weights = dict.fromkeys(labels, 0.0)
    for entry in content[0]["top_logprobs"]:
        token = str(entry["token"]).strip().lower()
        if not token:
            continue
        for label in labels:
            if label.startswith(token):
                weights[label] += math.exp(float(entry["logprob"]))
    total = sum(weights.values())
    return {label: weight / total for label, weight in weights.items()} if total > 0 else {}


def data_uri(frame: np.ndarray) -> str:
    ok, encoded = cv2.imencode(
        ".jpg", cv2.cvtColor(frame, cv2.COLOR_RGB2BGR), [cv2.IMWRITE_JPEG_QUALITY, JPEG_QUALITY]
    )
    if not ok:
        raise BadInput("a frame did not encode")
    return "data:image/jpeg;base64," + base64.b64encode(encoded.tobytes()).decode("ascii")


def text_option(spec: SlotSpec, name: str) -> str:
    value = spec.options.get(name)
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"{VLM_SLOT} needs the text option {name}")
    return value


def int_option(spec: SlotSpec, name: str) -> int:
    value = spec.options.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise RuntimeError(f"{VLM_SLOT} needs the integer option {name}")
    return value
