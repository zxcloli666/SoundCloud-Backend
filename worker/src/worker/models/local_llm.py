from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

METHOD = "generate"
QUANTIZE = "nf4"
WARMUP_PROMPT = "Reply with the JSON object {}."
WARMUP_TOKENS = 4
SYSTEM_PROMPT = (
    "You answer with exactly one JSON object and nothing else. "
    "The object must validate against this JSON Schema: "
)


class LocalLlmSlot:
    def __init__(self) -> None:
        self._model: Any = None
        self._tokenizer: Any = None

    def load(self, spec: SlotSpec) -> None:
        if not spec.device.startswith("cuda"):
            raise RuntimeError(f"llm-local runs only on CUDA, got device {spec.device!r}")
        quantize = spec.options.get("quantize")
        if quantize != QUANTIZE:
            raise RuntimeError(f"llm-local supports quantize={QUANTIZE!r}, got {quantize!r}")
        self._tokenizer = AutoTokenizer.from_pretrained(spec.model, revision=spec.revision)
        self._model = AutoModelForCausalLM.from_pretrained(
            spec.model,
            revision=spec.revision,
            quantization_config={
                "quant_method": "bitsandbytes",
                "load_in_4bit": True,
                "bnb_4bit_quant_type": QUANTIZE,
                "bnb_4bit_compute_dtype": "bfloat16",
            },
            device_map=spec.device,
        )
        self._model.eval()

    def warmup(self) -> None:
        self.generate(WARMUP_PROMPT, {"type": "object"}, WARMUP_TOKENS)

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != METHOD:
            raise ValueError(f"llm-local has no method {method!r}")
        prompt = args.get("prompt")
        schema = args.get("schema")
        max_new_tokens = args.get("max_new_tokens")
        if not isinstance(prompt, str) or not isinstance(schema, Mapping):
            raise BadInput("generate needs a string prompt and a schema object")
        if isinstance(max_new_tokens, bool) or not isinstance(max_new_tokens, int):
            raise BadInput("max_new_tokens must be an integer")
        return {}, {"text": self.generate(prompt, schema, max_new_tokens)}

    def unload(self) -> None:
        self._model = None
        self._tokenizer = None
        torch.cuda.empty_cache()

    def generate(self, prompt: str, schema: Mapping[str, object], max_new_tokens: int) -> str:
        if self._model is None:
            raise RuntimeError("llm-local slot is not loaded")
        messages = [
            {"role": "system", "content": SYSTEM_PROMPT + json.dumps(schema)},
            {"role": "user", "content": prompt},
        ]
        inputs = self._tokenizer.apply_chat_template(
            messages, add_generation_prompt=True, return_tensors="pt", return_dict=True
        ).to(self._model.device)
        with torch.inference_mode():
            output = self._model.generate(**inputs, max_new_tokens=max_new_tokens, do_sample=False)
        generated = output[0, inputs["input_ids"].shape[1] :]
        return json_span(str(self._tokenizer.decode(generated, skip_special_tokens=True)))


def json_span(text: str) -> str:
    start, end = text.find("{"), text.rfind("}")
    return text[start : end + 1] if 0 <= start < end else text.strip()
