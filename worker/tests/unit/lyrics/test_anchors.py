from __future__ import annotations

from itertools import pairwise

from hypothesis import given, settings
from hypothesis import strategies as st

from tests.unit.lyrics.conftest import RUSSIAN_LINES, lines_of
from worker.domain.lyrics import anchors, tokens
from worker.domain.lyrics.regions import Region
from worker.settings import AnchorSettings

ANCHORS = AnchorSettings(
    skip_region=0.15, skip_line=0.25, overflow=1.0, words_per_s=4.5, cjk_chars_per_s=8.0
)


def assigned(anchoring: anchors.Anchoring) -> list[tuple[int, ...]]:
    return [assignment.lines for assignment in anchoring.assignments]


def apart(durations: list[float], gap_s: float = 5.0) -> list[Region]:
    regions: list[Region] = []
    start = 0.0
    for duration in durations:
        regions.append(Region(start, start + duration))
        start += duration + gap_s
    return regions


def test_a_perfect_draft_maps_one_to_one() -> None:
    lines = lines_of(RUSSIAN_LINES)
    anchoring = anchors.assign(list(RUSSIAN_LINES), lines, apart([6.0] * 5), ANCHORS)
    assert assigned(anchoring) == [(0,), (1,), (2,), (3,), (4,)]
    assert anchoring.agreement > 0.95


def test_a_rough_draft_still_finds_the_right_line() -> None:
    drafts = [
        "закрой мне глаза чтобы я не видел тебя",
        "тело гниет зарастая в цветах",
        "просто забудь меня просто забудь",
        "мы сияем как в последний раз",
        "тебе с нами нельзя",
    ]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES), apart([6.0] * 5), ANCHORS)
    assert assigned(anchoring) == [(0,), (1,), (2,), (3,), (4,)]
    assert 0.6 < anchoring.agreement < 1.0


def test_a_region_holding_two_lines_gets_both() -> None:
    drafts = [RUSSIAN_LINES[0] + " " + RUSSIAN_LINES[1], RUSSIAN_LINES[2]]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES[:3]), apart([8.0, 6.0]), ANCHORS)
    assert assigned(anchoring) == [(0, 1), (2,)]


def test_a_region_with_foreign_text_is_skipped() -> None:
    drafts = [RUSSIAN_LINES[0], "ла ла ла ла ла инструментал", RUSSIAN_LINES[1]]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES[:2]), apart([6.0] * 3), ANCHORS)
    assert assigned(anchoring) == [(0,), (), (1,)]


def test_a_region_without_draft_takes_lines_by_capacity() -> None:
    drafts = [RUSSIAN_LINES[0], None, RUSSIAN_LINES[3]]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES[:4]), apart([6.0] * 3), ANCHORS)
    assert assigned(anchoring) == [(0,), (1, 2), (3,)]
    assert 1 not in anchoring.similarities


def test_lines_in_regions_without_draft_leave_the_agreement_alone() -> None:
    drafts = [RUSSIAN_LINES[0], None, None, None, RUSSIAN_LINES[4]]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES), apart([6.0] * 5), ANCHORS)
    assert assigned(anchoring) == [(0,), (1,), (2,), (3,), (4,)]
    assert anchoring.agreement > 0.95


def test_a_region_without_draft_is_never_free() -> None:
    lines = lines_of(RUSSIAN_LINES)
    keys = [anchors.line_key(line) for line in lines]
    seconds = [anchors.seconds_needed(line, ANCHORS) for line in lines]
    for scorer in (anchors.DraftMatcher, anchors.DraftSpelling):
        stretch = anchors.stretches([None], lines, apart([60.0]), 1, 0.0, scorer)[0][0]
        gains = anchors.group_gains(stretch, lines, keys, seconds, 0, ANCHORS)
        assert gains and all(gain < 0.0 for _, gain in gains)
        assert all(later < earlier for (_, earlier), (_, later) in pairwise(gains))


def test_repeated_chorus_lands_in_both_regions() -> None:
    texts = ["Куплет один совсем другой текст", "Припев поём вместе (x2)", "Финал песни тут"]
    drafts = [
        "куплет один совсем другой текст",
        "припев поём вместе",
        "припев поём вместе",
        "финал песни тут",
    ]
    anchoring = anchors.assign(drafts, lines_of(texts), apart([6.0] * 4), ANCHORS)
    assert assigned(anchoring) == [(0,), (1,), (2,), (3,)]


def test_a_misheard_draft_places_lines_by_spelling() -> None:
    drafts = [
        "Теплое место на улице ждут отпечатка нашего.",
        "Звездная пыль.",
        "Насарага.",
        "Накоёчка, ледчатый плет, для нажатой во время урок.",
    ]
    texts = [
        "Тёплое место, но улицы ждут",
        "Отпечатков наших ног",
        "Звёздная пыль на сапогах",
        "Мягкое кресло, клетчатый плед",
        "Не нажатый вовремя курок",
    ]
    anchoring = anchors.assign(drafts, lines_of(texts), apart([6.7, 2.4, 1.9, 6.7]), ANCHORS)
    assert assigned(anchoring) == [(0, 1), (2,), (), (3, 4)]
    assert anchoring.agreement < 0.3


def test_a_chorus_sung_once_hosts_one_copy_of_the_line() -> None:
    texts = [
        "Вступление другой текст",
        "Припев поём мы вместе",
        "Припев поём мы вместе",
        "Куплет второй другой",
        "Припев поём мы вместе",
    ]
    drafts = [
        "вступление другой текст",
        "припев поём мы вместе",
        "куплет второй другой",
        "припев поём мы вместе",
    ]
    anchoring = anchors.assign(drafts, lines_of(texts), apart([4.0] * 4), ANCHORS)
    assert [len(lines) for lines in assigned(anchoring)] == [1, 1, 1, 1]
    assert assigned(anchoring)[3] == (4,)


def test_a_line_sung_across_a_breath_takes_both_regions() -> None:
    texts = [
        "When you were here before couldn't look you in the eye",
        "You're just like an angel your skin makes me cry",
    ]
    drafts = [
        "When you were before.",
        "Couldn't look you in the eye.",
        "You're just like an angel. Your skin makes me cry.",
    ]
    regions = [Region(18.7, 20.9), Region(23.6, 26.2), Region(28.9, 34.0)]
    anchoring = anchors.assign(drafts, lines_of(texts, "en"), regions, ANCHORS)
    first = anchoring.assignments[0]
    assert (first.region, first.through, first.lines) == (0, 1, (0,))
    assert assigned(anchoring)[-1] == (1,)


def test_regions_far_apart_are_never_joined() -> None:
    texts = ["When you were here before couldn't look you in the eye"]
    drafts = ["When you were before.", "Couldn't look you in the eye."]
    regions = [Region(10.0, 12.0), Region(20.0, 23.0)]
    anchoring = anchors.assign(drafts, lines_of(texts, "en"), regions, ANCHORS)
    assert all(a.region == a.through for a in anchoring.assignments)


def test_cjk_capacity_counts_characters() -> None:
    texts = ["我的心里只有你没有他", "你要相信我的情意并不假"]
    lines = lines_of(texts, "zh")
    draft = ["我的心里只有你没有他 你要相信我的情意并不假"]
    assert assigned(anchors.assign(draft, lines, apart([3.0]), ANCHORS)) == [(0, 1)]
    assert assigned(anchors.assign(draft, lines, apart([1.0]), ANCHORS)) == [(0,)]
    assert anchors.seconds_needed(lines[0], ANCHORS) == 10 / 8.0


def test_foreign_lyrics_give_low_agreement() -> None:
    drafts = [
        "we walked along the river when the lights went down",
        "every word you never said still echoes",
        "hold me closer than the night",
        "the morning comes too early",
        "and the evening comes too slow",
    ]
    anchoring = anchors.assign(drafts, lines_of(RUSSIAN_LINES), apart([6.0] * 5), ANCHORS)
    assert anchoring.agreement < 0.3


def test_missing_input_yields_nothing() -> None:
    assert anchors.assign([], lines_of(RUSSIAN_LINES), [], ANCHORS).assignments == ()
    assert anchors.assign(["раз"], [], apart([1.0]), ANCHORS).assignments == ()


def score(line_text: str, draft: str, language: str = "ru") -> float:
    [line] = lines_of([line_text], language)
    return anchors.DraftMatcher(draft, [line]).single(anchors.line_key(line), language)


def test_line_score_is_the_share_of_its_bigrams_found_in_the_draft() -> None:
    assert score("зашей мне глаза", "зашей мне глаза") == 1.0
    assert score("зашей мне глаза", "ой зашей мне глаза чтоб") == 1.0
    assert score("зашей мне глаза", "зашей мне уши") == 0.5
    assert score("совсем другое", "зашей мне глаза") == 0.0
    assert score("мне", "зашей мне глаза") == 1.0
    assert score("зашей мне глаза", "") == 0.0


def test_korean_keys_ignore_spacing() -> None:
    line = lines_of(["사랑해요 여러분 안녕하세요"], "ko")[0]
    key = anchors.line_key(line)
    assert len(key.split()) == 12
    assert score("사랑해요 여러분 안녕하세요", "사랑해요여러분 안녕 하세요", "ko") == 1.0
    assert score("사랑해요 여러분 안녕하세요", "전혀 다른 가사입니다", "ko") == 0.0


def test_thai_keys_ignore_spacing() -> None:
    line = lines_of(["ฉันรักเธอ มากมาย"], "th")[0]
    key = anchors.line_key(line)
    assert key == tokens.similarity_key("ฉันรักเธอมากมาย", "th")
    assert anchors.DraftMatcher("ฉันรักเธอมากมาย", [line]).single(key, "th") == 1.0
    assert score("ฉันรักเธอ มากมาย", "สวัสดีครับ", "th") == 0.0


def test_english_words_glued_to_kana_still_match_their_line() -> None:
    line = lines_of(["my love"], "en")[0]
    key = anchors.line_key(line)
    assert anchors.DraftMatcher("君はmy loveだよ", [line]).single(key, "en") == 1.0


def test_line_keys_fold_case_and_diacritics() -> None:
    line = lines_of(["Тело гниёт, зарастая в цветах"])[0]
    assert anchors.line_key(line) == "тело гниет зарастая в цветах"
    assert score("Тело гниёт, зарастая в цветах", "тело гниет зарастая") == 0.5


@settings(max_examples=40, deadline=None)
@given(
    st.lists(
        st.sampled_from([*RUSSIAN_LINES, "ла ла ла", "", "мимо текста"]), min_size=1, max_size=8
    ),
    st.lists(st.floats(min_value=0.5, max_value=20.0), min_size=8, max_size=8),
)
def test_assignments_are_monotonic_and_tile_every_region(
    drafts: list[str], durations: list[float]
) -> None:
    lines = lines_of(RUSSIAN_LINES)
    anchoring = anchors.assign(
        [draft or None for draft in drafts], lines, apart(durations[: len(drafts)], 1.0), ANCHORS
    )
    covered = [
        region
        for assignment in anchoring.assignments
        for region in range(assignment.region, assignment.through + 1)
    ]
    assert covered == list(range(len(drafts)))
    assert all(a.through - a.region < anchors.MAX_JOINED_REGIONS for a in anchoring.assignments)
    order = [line for assignment in anchoring.assignments for line in assignment.lines]
    assert order == sorted(order)
    assert len(order) == len(set(order))
    assert all(0.0 <= value <= 1.0 for value in anchoring.similarities.values())
