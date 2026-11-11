from __future__ import annotations

from tests.fakes.llama import FakeModel
from worker.models.local_llm.prompt import Prompt, chat_template, plain_tokens

THINK = 1000
MARKER = "<think>"
TEMPLATE = chat_template(
    "{% for message in messages %}<{{ message.role }}>{{ message.content }}{% endfor %}<assistant>"
)
ROLE = len("<system>")


class Tokenizer(FakeModel):
    def __init__(self) -> None:
        super().__init__()
        self.markers = frozenset({THINK})

    def edge_texts(self) -> tuple[str, str]:
        return "", ""

    def tokenize(self, text: str, *, special: bool) -> list[int]:
        tokens: list[int] = []
        for index, part in enumerate(text.split(MARKER)):
            if index:
                tokens.append(THINK)
            tokens += [ord(letter) for letter in part]
        return tokens

    def piece(self, token: int) -> str:
        return MARKER if token == THINK else chr(token)


class Pairs(Tokenizer):
    def tokenize(self, text: str, *, special: bool) -> list[int]:
        pairs = [text[start : start + 2] for start in range(0, len(text), 2)]
        return [int.from_bytes(pair.encode("utf-8"), "big") for pair in pairs]


def user(text: str) -> dict[str, str]:
    return {"role": "user", "content": text}


def test_a_marker_token_typed_in_text_stays_text() -> None:
    model = Tokenizer()

    tokens = plain_tokens(model, f"a{MARKER}b")

    assert THINK not in tokens
    assert "".join(model.piece(token) for token in tokens) == f"a{MARKER}b"


def test_text_without_markers_is_tokenized_as_is() -> None:
    assert plain_tokens(Tokenizer(), "hello") == [ord(letter) for letter in "hello"]


def test_texts_with_the_same_bytes_up_to_a_mark_share_their_tokens_up_to_it() -> None:
    model = Pairs()
    asked = [user("hi")]

    marked = [Prompt.render(model, TEMPLATE, f"abc{tail}", asked, [3]) for tail in ("defg", "xyz")]
    plain = [Prompt.render(model, TEMPLATE, f"abc{tail}", asked) for tail in ("defg", "xyz")]

    head = ROLE // 2
    assert [prompt.marks for prompt in marked] == [(head + 2,), (head + 2,)]
    assert marked[0].tokens[: head + 2] == marked[1].tokens[: head + 2]
    assert [prompt.marks for prompt in plain] == [(), ()]
    assert plain[0].tokens[head : head + 4] == model.tokenize("abcdefg", special=False)
    assert plain[0].tokens[head + 1] != plain[1].tokens[head + 1]


def test_marks_count_bytes_and_cut_only_the_system_text() -> None:
    model = Pairs()
    text = "яблоко"
    whole = model.tokenize(text, special=False)

    prompt = Prompt.render(model, TEMPLATE, text, [user(text)], [6])

    assert prompt.marks == (ROLE // 2 + 2,)
    assert prompt.tokens[prompt.turn - len(whole) : prompt.turn] == whole


def test_a_mark_at_the_end_of_the_system_is_the_end_of_its_tokens() -> None:
    model = Pairs()

    marked = Prompt.render(model, TEMPLATE, "abcd", [user("hi")], [2, 4])
    plain = Prompt.render(model, TEMPLATE, "abcd", [user("hi")])

    assert marked.marks == (ROLE // 2 + 1, marked.shared)
    assert marked.tokens == plain.tokens


def test_a_marker_token_typed_in_a_marked_system_stays_text() -> None:
    model = Tokenizer()

    prompt = Prompt.render(model, TEMPLATE, f"ab{MARKER}cd", [user("hi")], [2 + len(MARKER)])

    assert THINK not in prompt.tokens
    assert prompt.marks == (ROLE + 2 + len(MARKER),)
