from __future__ import annotations

import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from itertools import pairwise

import jinja2
import jinja2.sandbox

from worker.models.local_llm.native import Model
from worker.runtime.protocol import BadInput

CONTENT_MARK = re.compile("([0-9]+)")


@dataclass(frozen=True)
class Prompt:
    tokens: list[int]
    shared: int
    turn: int
    marks: tuple[int, ...] = ()

    @classmethod
    def render(
        cls,
        model: Model,
        template: jinja2.Template,
        system: str,
        messages: Sequence[Mapping[str, str]],
        system_marks: Sequence[int] = (),
    ) -> Prompt:
        turns = [{"role": "system", "content": system}, *messages] if system else list(messages)
        marked = [
            {"role": turn["role"], "content": f"{index}"} for index, turn in enumerate(turns)
        ]
        bos, eos = model.edge_texts()
        try:
            text = template.render(
                messages=marked,
                add_generation_prompt=True,
                bos_token=bos,
                eos_token=eos,
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
        cuts: list[int] = []
        for position, piece in enumerate(pieces):
            if position % 2 == 0:
                tokens += model.tokenize(piece, special=True)
                continue
            content = turns[int(piece)]["content"]
            for part in split_at(content, system_marks if system and not ends else ()):
                tokens += plain_tokens(model, part)
                cuts.append(len(tokens))
            ends.append(cuts.pop())
        last = len(tokens) - 1
        shared = min(ends[0], last) if system else 0
        marks = tuple(cut for cut in cuts if cut <= shared)
        return cls(tokens, shared, min(ends[-1], last), marks)


def split_at(text: str, marks: Sequence[int]) -> list[str]:
    if not marks:
        return [text]
    encoded = text.encode("utf-8")
    edges = [0, *marks, len(encoded)]
    return [encoded[start:stop].decode("utf-8") for start, stop in pairwise(edges)]


def plain_tokens(model: Model, text: str) -> list[int]:
    tokens: list[int] = []
    for token in model.tokenize(text, special=False):
        if token in model.markers:
            piece = model.piece(token)
            tokens += model.tokenize(piece[:-1], special=False)
            tokens += model.tokenize(piece[-1:], special=False)
        else:
            tokens.append(token)
    return tokens


def chat_template(source: str) -> jinja2.Template:
    if not source:
        raise RuntimeError("the model file carries no chat template")
    environment = jinja2.sandbox.ImmutableSandboxedEnvironment(
        loader=jinja2.BaseLoader(), trim_blocks=True, lstrip_blocks=True
    )
    return environment.from_string(source)


class TemplateRefusal(Exception):
    pass


def refuse(message: str) -> None:
    raise TemplateRefusal(message)
