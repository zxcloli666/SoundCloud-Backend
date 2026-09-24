from __future__ import annotations

import os
from collections.abc import Iterator
from pathlib import Path

import numpy as np
import pytest

from tests.models.test_muq import MIN_REFERENCE_COSINE, reference, reference_windows, slot_spec
from worker.domain import index_audio
from worker.domain.audio import decode
from worker.domain.deadline import Deadline
from worker.runtime.protocol import BadInput

pytestmark = pytest.mark.models

TOP_K = 5
MIN_TOP_K_SHARE = 0.9
MAX_DURATION_S = 900.0
CAPTIONS = {
    "adele-someone-like-you-0918ab": "emotional piano ballad with female vocals",
    "nena-99-luftballons-00db33": "1980s new wave synth pop with german female vocals",
    "tarkan-simarik-81c8f2": "turkish pop with darbuka percussion",
    "daft-punk-voyager-5a49b9": "funky french house with disco guitar and synthesizers",
    "edith-piaf-la-vie-en-rose-7327b8": "vintage french chanson with orchestra, old recording",
    "eminem-rap-god-30c9d0": "fast aggressive hip hop rap with male vocals",
    "ludovico-einaudi-nuvole-bianche-82d9c9": "calm solo piano instrumental",
    "zhou-jie-lun-qing-hua-ci-7e78fc": "chinese pop ballad with traditional chinese instruments",
    "nirvana-smells-like-teen-spirit-90af6c": "grunge rock with distorted guitars and loud drums",
    "queen-bohemian-rhapsody-149a91": "operatic rock with piano and choir harmonies",
    "rammstein-sonne-b0c983": "industrial metal with heavy guitars and deep male german vocals",
    "pentatonix-hallelujah-95cb0c": "a cappella vocal group singing without instruments",
    "stromae-alors-on-danse-34738a": "minimal electronic dance beat with spoken male vocals",
    "shakira-hips-don-t-lie-765a58": "latin pop with trumpets and dance rhythm",
    "oasis-wonderwall-795bfa": "britpop with strummed acoustic guitar and male vocals",
    "amr-diab-tamally-maak-cac88a": "arabic pop with middle eastern percussion",
    "arijit-singh-tum-hi-ho-614bed": "romantic bollywood ballad with male vocals",
    "rosalia-malamente-985624": "modern flamenco with hand claps and female vocals",
    "yoasobi-ye-niqu-keru-9dc25a": "upbeat japanese pop with fast female vocals",
    "korol-i-shut-kukla-kolduna-3f35d5": "russian punk rock with accordion-like melody",
}


@pytest.fixture(scope="module")
def mulan() -> Iterator[object]:
    import torch

    from worker.models.mulan import MulanSlot

    slot = MulanSlot()
    slot.load(slot_spec("mulan", "worker.models.mulan:MulanSlot"))
    slot.warmup()
    yield slot
    slot.unload()
    torch.cuda.empty_cache()


def audio(slot: object, windows: np.ndarray) -> np.ndarray:
    arrays, _ = slot.invoke("embed_audio", {"windows": windows}, {})
    return arrays["vectors"]


def text(slot: object, texts: list[str]) -> np.ndarray:
    arrays, _ = slot.invoke("embed_text", {}, {"texts": texts})
    return arrays["vectors"]


def test_audio_matches_prototype_reference(mulan: object) -> None:
    vector = audio(mulan, reference_windows())[0]

    assert vector.shape == (512,)
    assert float(np.dot(vector, reference()["mulan_audio"])) >= MIN_REFERENCE_COSINE


def test_text_matches_prototype_reference(mulan: object) -> None:
    expected = reference()["mulan_text"]

    vectors = text(mulan, [str(item) for item in reference()["texts"]])

    assert vectors.shape == (2, 512)
    for vector, reference_vector in zip(vectors, expected, strict=True):
        assert float(np.dot(vector, reference_vector)) >= MIN_REFERENCE_COSINE


def test_vectors_are_unit_512(mulan: object) -> None:
    clip = reference_windows()[0]
    windows = np.stack([clip, 0.5 * clip])

    rows = np.concatenate([audio(mulan, windows), text(mulan, ["sad piano", "техно на рейве"])])

    assert rows.shape == (4, 512)
    assert rows.dtype == np.float32
    assert np.allclose(np.linalg.norm(rows, axis=1), 1.0, atol=1e-5)


async def test_captions_find_their_tracks_in_top_five(mulan: object) -> None:
    tracks = [await track_vector(mulan, name) for name in CAPTIONS]
    captions = text(mulan, list(CAPTIONS.values()))

    similarity = captions @ np.stack(tracks).T
    ranks = [int((row > row[index]).sum()) for index, row in enumerate(similarity)]

    assert np.mean(np.array(ranks) < TOP_K) >= MIN_TOP_K_SHARE, ranks


async def track_vector(slot: object, name: str) -> np.ndarray:
    data = os.environ.get("EVAL_DATA_DIR")
    if not data:
        pytest.skip("needs the eval audio (EVAL_DATA_DIR)")
    path = Path(data) / "audio" / f"{name}.m4a"
    pcm = await decode.decode(path, max_duration_s=MAX_DURATION_S, deadline=Deadline.after(120))
    clips = index_audio.embedding_clips(decode.to_mono(pcm), pcm.sample_rate)
    mean = audio(slot, clips).mean(axis=0)
    return mean / np.linalg.norm(mean)


@pytest.mark.parametrize(
    "args",
    [{}, {"texts": []}, {"texts": "rock"}, {"texts": [" "]}, {"texts": ["a " * 600]}],
)
def test_bad_texts(mulan: object, args: dict[str, object]) -> None:
    with pytest.raises(BadInput):
        mulan.invoke("embed_text", {}, args)


def test_unknown_method(mulan: object) -> None:
    with pytest.raises(BadInput):
        mulan.invoke("embed", {}, {})
