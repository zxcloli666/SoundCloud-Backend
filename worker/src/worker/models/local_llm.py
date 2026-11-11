from __future__ import annotations

import ctypes
import json
import re
import time
from collections import OrderedDict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass

import jinja2
import jinja2.sandbox
import llama_cpp
import numpy as np
from huggingface_hub import hf_hub_download
from llama_cpp.llama_grammar import json_schema_to_gbnf

from worker.runtime.protocol import CALL_GUARD, Arrays, BadInput, CallExpired, SlotSpec

METHOD = "complete"
ROLES = frozenset({"user", "assistant"})
EVAL_CHUNK_TOKENS = 128
SEQUENCE = 0
CPU_ONLY_LAYERS = 0
TEMPLATE_KEY = "tokenizer.chat_template"
GRAMMAR_ROOT = b"root"
CONTENT_MARK = re.compile("\ue000([0-9]+)\ue001")
TOKEN_DATA = np.dtype([("id", np.intc), ("logit", np.single), ("p", np.single)], align=True)
WARMUP_SYSTEM = "You answer with one JSON object."
WARMUP_MESSAGES = ({"role": "user", "content": "Are you ready?"},)
WARMUP_SCHEMA: Mapping[str, object] = {
    "type": "object",
    "properties": {"ok": {"const": True}},
    "required": ["ok"],
    "additionalProperties": False,
}
WARMUP_TOKENS = 16


class LocalLlmSlot:
    def __init__(self) -> None:
        self._llm: llama_cpp.Llama | None = None
        self._template: jinja2.Template | None = None
        self._states = PromptStates(0)
        self._weights: Weights = ("", 0)
        self._reading = Pace()
        self._writing = Pace()
        self._threads = 0

    def load(self, spec: SlotSpec) -> None:
        file = text_option(spec, "file")
        context_tokens = count_option(spec, "context_tokens")
        path = hf_hub_download(spec.model, file, revision=spec.revision, local_files_only=True)
        self._weights = (path, context_tokens)
        survived = SURVIVING_STATES.pop(self._weights, None)
        self._states = survived or PromptStates(count_option(spec, "prompt_states"))
        self._threads = CALL_GUARD.threads
        self._llm = llama_cpp.Llama(
            model_path=path,
            n_ctx=context_tokens,
            n_batch=EVAL_CHUNK_TOKENS,
            n_threads=self._threads or None,
            n_threads_batch=self._threads or None,
            n_gpu_layers=CPU_ONLY_LAYERS,
            verbose=False,
        )
        self._template = chat_template(self._llm)

    def warmup(self) -> None:
        self.complete(WARMUP_SYSTEM, WARMUP_MESSAGES, WARMUP_SCHEMA, WARMUP_TOKENS)

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != METHOD:
            raise ValueError(f"llm-local has no method {method!r}")
        system = args.get("system")
        schema = args.get("schema")
        max_tokens = args.get("max_tokens")
        if not isinstance(system, str) or not isinstance(schema, Mapping):
            raise BadInput("complete needs a string system and a schema object")
        if isinstance(max_tokens, bool) or not isinstance(max_tokens, int) or max_tokens < 1:
            raise BadInput("max_tokens must be a positive integer")
        return {}, self.complete(system, parse_messages(args.get("messages")), schema, max_tokens)

    def unload(self) -> None:
        if self._llm is not None:
            self._llm.close()
            self._states.forget_turns()
            SURVIVING_STATES[self._weights] = self._states
        self._llm = None
        self._template = None
        self._states = PromptStates(0)

    def complete(
        self,
        system: str,
        messages: Sequence[Mapping[str, str]],
        schema: Mapping[str, object],
        max_tokens: int,
    ) -> Mapping[str, object]:
        if self._llm is None or self._template is None:
            raise RuntimeError("llm-local slot is not loaded")
        llm = self._llm
        prompt = Prompt.render(llm, self._template, system, messages)
        if len(prompt.tokens) + max_tokens > llm.n_ctx():
            raise BadInput(
                f"a prompt of {len(prompt.tokens)} tokens and an answer of {max_tokens} tokens "
                f"do not fit the context of {llm.n_ctx()} tokens"
            )
        grammar = answer_grammar(llm, schema)
        try:
            self._follow_call_threads(llm)
            reused = read(llm, self._states, prompt, self._reading)
            answer, finished = write(llm, prompt.tokens[-1], grammar, max_tokens, self._writing)
        finally:
            llama_cpp.llama_sampler_free(grammar)
        return {
            "text": llm.detokenize(answer).decode("utf-8", errors="replace"),
            "input_tokens": len(prompt.tokens),
            "reused_tokens": reused,
            "output_tokens": len(answer),
            "finished": finished,
        }

    def _follow_call_threads(self, llm: llama_cpp.Llama) -> None:
        threads = CALL_GUARD.threads
        if threads < 1 or threads == self._threads:
            return
        llama_cpp.llama_set_n_threads(llm.ctx, threads, threads)
        self._threads = threads


def text_option(spec: SlotSpec, name: str) -> str:
    value = spec.options.get(name)
    if not isinstance(value, str):
        raise RuntimeError(f"llm-local needs the text option {name}")
    return value


def count_option(spec: SlotSpec, name: str) -> int:
    value = spec.options.get(name)
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        raise RuntimeError(f"llm-local needs the positive integer option {name}")
    return value


def chat_template(llm: llama_cpp.Llama) -> jinja2.Template:
    source = llm.metadata.get(TEMPLATE_KEY)
    if not source:
        raise RuntimeError(f"the model file carries no {TEMPLATE_KEY}")
    environment = jinja2.sandbox.ImmutableSandboxedEnvironment(
        loader=jinja2.BaseLoader(), trim_blocks=True, lstrip_blocks=True
    )
    return environment.from_string(source)


def parse_messages(messages: object) -> list[Mapping[str, str]]:
    if not isinstance(messages, list) or not messages:
        raise BadInput("complete needs a non-empty list of messages")
    return [parse_message(message) for message in messages]


def parse_message(message: object) -> Mapping[str, str]:
    if not isinstance(message, Mapping):
        raise BadInput("a message must be an object")
    role, content = message.get("role"), message.get("content")
    if role not in ROLES or not isinstance(content, str):
        raise BadInput(f"a message needs a role of {sorted(ROLES)} and a string content")
    return {"role": str(role), "content": content}


@dataclass(frozen=True)
class Prompt:
    tokens: list[int]
    shared: int
    turn: int

    @classmethod
    def render(
        cls,
        llm: llama_cpp.Llama,
        template: jinja2.Template,
        system: str,
        messages: Sequence[Mapping[str, str]],
    ) -> Prompt:
        turns = [{"role": "system", "content": system}, *messages] if system else list(messages)
        marked = [
            {"role": turn["role"], "content": f"\ue000{index}\ue001"}
            for index, turn in enumerate(turns)
        ]
        try:
            text = template.render(
                messages=marked,
                add_generation_prompt=True,
                bos_token=special_text(llm, llm.token_bos()),
                eos_token=special_text(llm, llm.token_eos()),
                enable_thinking=False,
                raise_exception=refuse,
            )
        except (jinja2.TemplateError, TemplateRefusal) as error:
            raise BadInput(f"the chat template refuses these messages: {error}") from error
        pieces = CONTENT_MARK.split(text)
        if [int(mark) for mark in pieces[1::2]] != list(range(len(turns))):
            raise RuntimeError("the chat template does not place every message once and in order")
        tokens: list[int] = []
        ends: list[int] = []
        for position, piece in enumerate(pieces):
            written = position % 2 == 0
            source = piece if written else turns[int(piece)]["content"]
            tokens += llm.tokenize(source.encode("utf-8"), add_bos=False, special=written)
            if not written:
                ends.append(len(tokens))
        last = len(tokens) - 1
        return cls(tokens, min(ends[0], last) if system else 0, min(ends[-1], last))


def special_text(llm: llama_cpp.Llama, token: int) -> str:
    if token < 0:
        return ""
    return llm.detokenize([token], special=True).decode("utf-8", errors="replace")


class TemplateRefusal(Exception):
    pass


def refuse(message: str) -> None:
    raise TemplateRefusal(message)


def answer_grammar(llm: llama_cpp.Llama, schema: Mapping[str, object]) -> llama_cpp.llama_sampler_p:
    try:
        rules = json_schema_to_gbnf(json.dumps(schema))
    except Exception as error:
        raise BadInput(f"the schema cannot constrain an answer: {error!r}") from error
    vocab = llama_cpp.llama_model_get_vocab(llm.model)
    grammar: llama_cpp.llama_sampler_p = llama_cpp.llama_sampler_init_grammar(
        vocab, rules.encode("utf-8"), GRAMMAR_ROOT
    )
    if not grammar:
        raise BadInput("the schema does not make a usable grammar")
    return grammar


def read(llm: llama_cpp.Llama, states: PromptStates, prompt: Prompt, pace: Pace) -> int:
    tokens = prompt.tokens
    stable = states.stable_prefix(tokens, prompt.shared)
    reused = restore(llm, states, tokens)
    try:
        for mark, anchor in ((stable, True), (prompt.shared, True), (prompt.turn, False)):
            if llm.n_tokens < mark:
                evaluate(llm, tokens, mark, pace)
                states.keep(tokens[:mark], snapshot(llm), anchor)
        evaluate(llm, tokens, len(tokens) - 1, pace)
    except CallExpired:
        if llm.n_tokens > reused:
            states.keep(tokens[: llm.n_tokens], snapshot(llm), False)
        raise
    return reused


def restore(llm: llama_cpp.Llama, states: PromptStates, tokens: Sequence[int]) -> int:
    forget(llm)
    held = states.deepest(tokens)
    if held is None:
        return 0
    prefix, state = held
    if llama_cpp.llama_state_seq_set_data(llm.ctx, state, len(state), SEQUENCE) != len(state):
        states.drop(prefix)
        forget(llm)
        raise RuntimeError("a saved prompt state did not load")
    llm.n_tokens = len(prefix)
    return llm.n_tokens


def forget(llm: llama_cpp.Llama) -> None:
    llama_cpp.llama_memory_clear(llama_cpp.llama_get_memory(llm.ctx), True)
    llm.n_tokens = 0


def evaluate(llm: llama_cpp.Llama, tokens: Sequence[int], stop: int, pace: Pace) -> None:
    for start in range(llm.n_tokens, stop, EVAL_CHUNK_TOKENS):
        pace.eval(llm, tokens[start : min(start + EVAL_CHUNK_TOKENS, stop)])


def snapshot(llm: llama_cpp.Llama) -> State:
    size = llama_cpp.llama_state_seq_get_size(llm.ctx, SEQUENCE)
    state: State = (ctypes.c_uint8 * size)()
    if llama_cpp.llama_state_seq_get_data(llm.ctx, state, size, SEQUENCE) != size:
        raise RuntimeError("the prompt state did not save")
    return state


State = ctypes.Array[ctypes.c_uint8]
Prefix = tuple[int, ...]
Weights = tuple[str, int]


@dataclass(frozen=True)
class Held:
    state: State
    anchor: bool


class PromptStates:
    def __init__(self, capacity: int) -> None:
        self._capacity = capacity
        self._held: OrderedDict[Prefix, Held] = OrderedDict()

    def sizes(self) -> list[int]:
        return [len(prefix) for prefix in self._held]

    def deepest(self, tokens: Sequence[int]) -> tuple[Prefix, State] | None:
        found = sorted((prefix for prefix in self._held if starts_with(tokens, prefix)), key=len)
        for prefix in found:
            self._held.move_to_end(prefix)
        return (found[-1], self._held[found[-1]].state) if found else None

    def stable_prefix(self, tokens: Sequence[int], shared: int) -> int:
        common = max((common_length(tokens, prefix) for prefix in self._held), default=0)
        return common if EVAL_CHUNK_TOKENS <= common < shared else 0

    def keep(self, tokens: Sequence[int], state: State, anchor: bool) -> None:
        if len(tokens) < EVAL_CHUNK_TOKENS:
            return
        prefix = tuple(tokens)
        outgrown = [
            held
            for held, entry in self._held.items()
            if not entry.anchor and starts_with(prefix, held)
        ]
        for held in outgrown:
            del self._held[held]
        self._held[prefix] = Held(state, anchor)
        self._held.move_to_end(prefix)
        while len(self._held) > self._capacity:
            self._held.popitem(last=False)

    def drop(self, prefix: Prefix) -> None:
        del self._held[prefix]

    def forget_turns(self) -> None:
        for prefix in [prefix for prefix, held in self._held.items() if not held.anchor]:
            del self._held[prefix]


SURVIVING_STATES: dict[Weights, PromptStates] = {}


def starts_with(tokens: Sequence[int], prefix: Prefix) -> bool:
    return len(prefix) < len(tokens) and tuple(tokens[: len(prefix)]) == prefix


def common_length(tokens: Sequence[int], prefix: Prefix) -> int:
    length = 0
    for ours, theirs in zip(tokens, prefix, strict=False):
        if ours != theirs:
            break
        length += 1
    return length


def write(
    llm: llama_cpp.Llama,
    last: int,
    grammar: llama_cpp.llama_sampler_p,
    max_tokens: int,
    pace: Pace,
) -> tuple[list[int], bool]:
    vocab = llama_cpp.llama_model_get_vocab(llm.model)
    answer: list[int] = []
    pending = last
    while True:
        pace.eval(llm, [pending])
        token = pick(llm, grammar)
        if llama_cpp.llama_vocab_is_eog(vocab, token):
            return answer, True
        if len(answer) == max_tokens:
            return answer, False
        llama_cpp.llama_sampler_accept(grammar, token)
        answer.append(token)
        pending = token


class Pace:
    def __init__(self) -> None:
        self._seconds_per_token = 0.0

    def eval(self, llm: llama_cpp.Llama, tokens: Sequence[int]) -> None:
        CALL_GUARD.check(len(tokens) * self._seconds_per_token)
        started = time.monotonic()
        llm.eval(tokens)
        self._seconds_per_token = (time.monotonic() - started) / len(tokens)


def pick(llm: llama_cpp.Llama, grammar: llama_cpp.llama_sampler_p) -> int:
    size = llm.n_vocab()
    logits = np.ctypeslib.as_array(llama_cpp.llama_get_logits_ith(llm.ctx, -1), shape=(size,))
    favourite = np.zeros(1, dtype=TOKEN_DATA)
    favourite["id"] = logits.argmax()
    constrain(grammar, favourite)
    if np.isfinite(favourite["logit"][0]):
        return int(favourite["id"][0])
    candidates = np.zeros(size, dtype=TOKEN_DATA)
    candidates["id"] = np.arange(size)
    candidates["logit"] = logits
    constrain(grammar, candidates)
    return int(candidates["logit"].argmax())


def constrain(grammar: llama_cpp.llama_sampler_p, candidates: np.ndarray) -> None:
    array = llama_cpp.llama_token_data_array(
        data=candidates.ctypes.data_as(llama_cpp.llama_token_data_p),
        size=len(candidates),
        selected=-1,
        sorted=False,
    )
    llama_cpp.llama_sampler_apply(grammar, ctypes.byref(array))
