from __future__ import annotations

import ctypes
import json
from collections.abc import Mapping, Sequence

import llama_cpp
import numpy as np
from llama_cpp.llama_grammar import json_schema_to_gbnf

from worker.runtime.protocol import BadInput

SEQUENCE = 0
CPU_ONLY_LAYERS = 0
TEMPLATE_KEY = "tokenizer.chat_template"
GRAMMAR_ROOT = b"root"
TOKEN_DATA = np.dtype([("id", np.intc), ("logit", np.single), ("p", np.single)], align=True)

State = ctypes.Array[ctypes.c_uint8]


class Model:
    def __init__(self, path: str, context_tokens: int, chunk_tokens: int, threads: int) -> None:
        self._llm = llama_cpp.Llama(
            model_path=path,
            n_ctx=context_tokens,
            n_batch=chunk_tokens,
            n_threads=threads or None,
            n_threads_batch=threads or None,
            n_gpu_layers=CPU_ONLY_LAYERS,
            verbose=False,
        )
        self.vocab = llama_cpp.llama_model_get_vocab(self._llm.model)
        self.threads = threads
        self.markers = frozenset(
            token
            for token in range(self._llm.n_vocab())
            if llama_cpp.llama_vocab_get_attr(self.vocab, token)
            & llama_cpp.LLAMA_TOKEN_ATTR_USER_DEFINED
        )

    @property
    def context_tokens(self) -> int:
        return self._llm.n_ctx()

    @property
    def read_tokens(self) -> int:
        return self._llm.n_tokens

    @property
    def vocab_size(self) -> int:
        return self._llm.n_vocab()

    def template_source(self) -> str:
        return str(self._llm.metadata.get(TEMPLATE_KEY, ""))

    def edge_texts(self) -> tuple[str, str]:
        return self.piece(self._llm.token_bos()), self.piece(self._llm.token_eos())

    def tokenize(self, text: str, *, special: bool) -> list[int]:
        return self._llm.tokenize(text.encode("utf-8"), add_bos=False, special=special)

    def piece(self, token: int) -> str:
        if token < 0:
            return ""
        return self._llm.detokenize([token], special=True).decode("utf-8", errors="replace")

    def detokenize(self, tokens: Sequence[int]) -> str:
        return self._llm.detokenize(list(tokens)).decode("utf-8", errors="replace")

    def eval(self, tokens: Sequence[int]) -> None:
        self._llm.eval(tokens)

    def clear(self) -> None:
        llama_cpp.llama_memory_clear(llama_cpp.llama_get_memory(self._llm.ctx), True)
        self._llm.n_tokens = 0

    def save(self) -> State:
        size = llama_cpp.llama_state_seq_get_size(self._llm.ctx, SEQUENCE)
        state: State = (ctypes.c_uint8 * size)()
        if llama_cpp.llama_state_seq_get_data(self._llm.ctx, state, size, SEQUENCE) != size:
            raise RuntimeError("the prompt state did not save")
        return state

    def restore(self, state: State, tokens: int) -> bool:
        loaded = llama_cpp.llama_state_seq_set_data(self._llm.ctx, state, len(state), SEQUENCE)
        if loaded != len(state):
            self.clear()
            return False
        self._llm.n_tokens = tokens
        return True

    def logits(self) -> np.ndarray:
        last = llama_cpp.llama_get_logits_ith(self._llm.ctx, -1)
        return np.ctypeslib.as_array(last, shape=(self.vocab_size,))

    def is_end(self, token: int) -> bool:
        return bool(llama_cpp.llama_vocab_is_eog(self.vocab, token))

    def follow_threads(self, threads: int) -> None:
        if threads < 1 or threads == self.threads:
            return
        llama_cpp.llama_set_n_threads(self._llm.ctx, threads, threads)
        self.threads = threads

    def close(self) -> None:
        self._llm.close()


class Grammar:
    def __init__(self, model: Model, schema: Mapping[str, object]) -> None:
        try:
            rules = json_schema_to_gbnf(json.dumps(schema))
        except Exception as error:
            raise BadInput(f"the schema cannot constrain an answer: {error!r}") from error
        self._sampler: llama_cpp.llama_sampler_p = llama_cpp.llama_sampler_init_grammar(
            model.vocab, rules.encode("utf-8"), GRAMMAR_ROOT
        )
        if not self._sampler:
            raise BadInput("the schema does not make a usable grammar")

    def constrain(self, candidates: np.ndarray) -> None:
        array = llama_cpp.llama_token_data_array(
            data=candidates.ctypes.data_as(llama_cpp.llama_token_data_p),
            size=len(candidates),
            selected=-1,
            sorted=False,
        )
        llama_cpp.llama_sampler_apply(self._sampler, ctypes.byref(array))

    def accept(self, token: int) -> None:
        llama_cpp.llama_sampler_accept(self._sampler, token)

    def close(self) -> None:
        llama_cpp.llama_sampler_free(self._sampler)
