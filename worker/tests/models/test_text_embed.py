from __future__ import annotations

from collections.abc import Iterator

import numpy as np
import pytest

from tests.models.test_muq import slot_spec
from worker.runtime.protocol import BadInput

pytestmark = pytest.mark.models

DIM = 1024
MIN_TRANSLATION_GAP = 0.15
TRANSLATIONS = [
    ("песня о любви", "a song about love"),
    ("я иду домой под дождём", "I am walking home in the rain"),
    ("мы танцуем до утра", "we dance until the morning"),
]
UNRELATED = ["quarterly tax report", "how to replace a car tyre", "database index tuning"]


@pytest.fixture(scope="module")
def text_slot() -> Iterator[object]:
    import torch

    from worker.models.text_embed import TextEmbedSlot

    slot = TextEmbedSlot()
    slot.load(slot_spec("text", "worker.models.text_embed:TextEmbedSlot"))
    slot.warmup()
    yield slot
    slot.unload()
    torch.cuda.empty_cache()


def embed(slot: object, texts: list[str], kind: str = "document") -> np.ndarray:
    arrays, _ = slot.invoke("embed", {}, {"texts": texts, "kind": kind})
    return arrays["vectors"]


@pytest.mark.parametrize("kind", ["document", "query"])
def test_vectors_are_unit_1024(text_slot: object, kind: str) -> None:
    vectors = embed(text_slot, ["короткая строка", "a longer line of english lyrics"], kind)

    assert vectors.shape == (2, DIM)
    assert vectors.dtype == np.float32
    assert np.all(np.isfinite(vectors))
    assert np.allclose(np.linalg.norm(vectors, axis=1), 1.0, atol=1e-3)


def test_translation_is_closer_than_unrelated_text(text_slot: object) -> None:
    unrelated = embed(text_slot, UNRELATED)
    for russian, english in TRANSLATIONS:
        pair = embed(text_slot, [russian, english])

        translated = float(pair[0] @ pair[1])
        nearest_unrelated = float((pair @ unrelated.T).max())

        assert translated - nearest_unrelated >= MIN_TRANSLATION_GAP, (russian, english)


def test_query_prompt_changes_the_vector(text_slot: object) -> None:
    line = ["песня о любви"]

    document = embed(text_slot, line, "document")[0]
    query = embed(text_slot, line, "query")[0]

    assert float(document @ query) < 0.99


def test_batch_matches_single_rows(text_slot: object) -> None:
    texts = ["первая строка", "second line of the song", "третья"]

    batch = embed(text_slot, texts)
    singles = np.stack([embed(text_slot, [line])[0] for line in texts])

    assert np.all(np.sum(batch * singles, axis=1) >= 0.999)


def test_text_over_token_limit_is_bad_input(text_slot: object) -> None:
    with pytest.raises(BadInput, match="limit 8192"):
        embed(text_slot, ["слово " * 9000])


@pytest.mark.parametrize(
    "args",
    [
        {"texts": ["x"]},
        {"texts": ["x"], "kind": "passage"},
        {"texts": [], "kind": "query"},
        {"texts": "x", "kind": "query"},
        {"texts": [1], "kind": "query"},
    ],
)
def test_bad_arguments(text_slot: object, args: dict[str, object]) -> None:
    with pytest.raises(BadInput):
        text_slot.invoke("embed", {}, args)


def test_unknown_method(text_slot: object) -> None:
    with pytest.raises(BadInput):
        text_slot.invoke("encode", {}, {"texts": ["x"], "kind": "query"})
