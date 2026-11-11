from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass

import jinja2
from huggingface_hub import hf_hub_download

from worker.models.local_llm.native import Grammar, Model, State
from worker.models.local_llm.pace import Pace
from worker.models.local_llm.prompt import Prompt, chat_template
from worker.models.local_llm.reading import read
from worker.models.local_llm.sampling import write
from worker.models.local_llm.states import PromptStates
from worker.runtime.protocol import CALL_GUARD, Arrays, BadInput, SlotSpec
from worker.settings import LOCAL_LLM_METHOD, LOCAL_LLM_SLOT

MIB = 1 << 20
READ_SAMPLE_TOKENS = 16
WARMUP_SENTENCE = "You answer with one JSON object. "
WARMUP_MESSAGES = ({"role": "user", "content": "Are you ready?"},)
WARMUP_SCHEMA: Mapping[str, object] = {
    "type": "object",
    "properties": {"ok": {"const": True}},
    "required": ["ok"],
    "additionalProperties": False,
}
WARMUP_TOKENS = 16


@dataclass(frozen=True)
class Options:
    file: str
    context_tokens: int
    chunk_tokens: int
    state_cache_mib: int

    @classmethod
    def of(cls, spec: SlotSpec) -> Options:
        file = spec.options.get("file")
        if not isinstance(file, str):
            raise RuntimeError(f"{LOCAL_LLM_SLOT} needs the text option file")
        return cls(
            file,
            count_option(spec, "context_tokens"),
            count_option(spec, "chunk_tokens"),
            count_option(spec, "state_cache_mib"),
        )


class LocalLlmSlot:
    def __init__(self) -> None:
        self._model: Model | None = None
        self._template: jinja2.Template | None = None
        self._source: tuple[str, Options] | None = None
        self._states: PromptStates[State] = PromptStates(0, 1)
        self._reading = Pace(1)
        self._writing = Pace(1)

    def load(self, spec: SlotSpec) -> None:
        options = Options.of(spec)
        path = hf_hub_download(
            spec.model, options.file, revision=spec.revision, local_files_only=True
        )
        if self._source != (path, options):
            self._source = (path, options)
            self._states = PromptStates(options.state_cache_mib * MIB, options.chunk_tokens)
            self._reading = Pace(min(options.chunk_tokens, READ_SAMPLE_TOKENS))
        self._model = Model(path, options.context_tokens, options.chunk_tokens, CALL_GUARD.threads)
        self._template = chat_template(self._model.template_source())

    def warmup(self) -> None:
        if self._model is None or self._source is None:
            raise RuntimeError(f"{LOCAL_LLM_SLOT} is not loaded")
        if self._reading.known:
            return
        chunk_tokens = self._source[1].chunk_tokens
        system = WARMUP_SENTENCE
        while len(self._model.tokenize(system, special=False)) < chunk_tokens:
            system += WARMUP_SENTENCE
        forgotten: PromptStates[State] = PromptStates(0, chunk_tokens)
        self._answer(system, (), WARMUP_MESSAGES, WARMUP_SCHEMA, WARMUP_TOKENS, forgotten)

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != LOCAL_LLM_METHOD:
            raise ValueError(f"{LOCAL_LLM_SLOT} has no method {method!r}")
        system, marks = args.get("system"), args.get("system_marks")
        messages = args.get("messages")
        schema, max_tokens = args.get("schema"), args.get("max_tokens")
        if not (
            isinstance(system, str)
            and isinstance(marks, list)
            and isinstance(messages, list)
            and isinstance(schema, Mapping)
            and isinstance(max_tokens, int)
        ):
            raise TypeError(f"{LOCAL_LLM_METHOD} got arguments of the wrong types")
        return {}, self.complete(system, marks, messages, schema, max_tokens)

    def unload(self) -> None:
        if self._model is not None:
            self._model.close()
            self._states.forget_turns()
        self._model = None
        self._template = None

    def complete(
        self,
        system: str,
        system_marks: Sequence[int],
        messages: Sequence[Mapping[str, str]],
        schema: Mapping[str, object],
        max_tokens: int,
    ) -> Mapping[str, object]:
        return self._answer(system, system_marks, messages, schema, max_tokens, self._states)

    def _answer(
        self,
        system: str,
        system_marks: Sequence[int],
        messages: Sequence[Mapping[str, str]],
        schema: Mapping[str, object],
        max_tokens: int,
        states: PromptStates[State],
    ) -> Mapping[str, object]:
        if self._model is None or self._template is None or self._source is None:
            raise RuntimeError(f"{LOCAL_LLM_SLOT} is not loaded")
        model, chunk_tokens = self._model, self._source[1].chunk_tokens
        prompt = Prompt.render(model, self._template, system, messages, system_marks)
        if len(prompt.tokens) + max_tokens > model.context_tokens:
            raise BadInput(
                f"a prompt of {len(prompt.tokens)} tokens and an answer of {max_tokens} tokens "
                f"do not fit the context of {model.context_tokens} tokens"
            )
        grammar = Grammar(model, schema)
        try:
            model.follow_threads(CALL_GUARD.threads)
            reused = read(model, states, prompt, self._reading, chunk_tokens)
            answer, finished = write(model, prompt.tokens[-1], grammar, max_tokens, self._writing)
        finally:
            grammar.close()
        return {
            "text": model.detokenize(answer),
            "input_tokens": len(prompt.tokens),
            "reused_tokens": reused,
            "output_tokens": len(answer),
            "finished": finished,
        }


def count_option(spec: SlotSpec, name: str) -> int:
    value = spec.options.get(name)
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise RuntimeError(f"{LOCAL_LLM_SLOT} needs the positive integer option {name}")
    return value
