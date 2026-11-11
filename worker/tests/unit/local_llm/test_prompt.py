from __future__ import annotations

from tests.fakes.llama import FakeModel
from worker.models.local_llm.prompt import plain_tokens

THINK = 1000
MARKER = "<think>"


class Tokenizer(FakeModel):
    def __init__(self) -> None:
        super().__init__()
        self.markers = frozenset({THINK})

    def tokenize(self, text: str, *, special: bool) -> list[int]:
        tokens: list[int] = []
        for index, part in enumerate(text.split(MARKER)):
            if index:
                tokens.append(THINK)
            tokens += [ord(letter) for letter in part]
        return tokens

    def piece(self, token: int) -> str:
        return MARKER if token == THINK else chr(token)


def test_a_marker_token_typed_in_text_stays_text() -> None:
    model = Tokenizer()

    tokens = plain_tokens(model, f"a{MARKER}b")

    assert THINK not in tokens
    assert "".join(model.piece(token) for token in tokens) == f"a{MARKER}b"


def test_text_without_markers_is_tokenized_as_is() -> None:
    assert plain_tokens(Tokenizer(), "hello") == [ord(letter) for letter in "hello"]
