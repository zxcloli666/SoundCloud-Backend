from __future__ import annotations

from worker.models.local_llm.states import PromptStates

CHUNK = 4


def tokens(head: int, length: int) -> list[int]:
    return [head, *range(100, 100 + length - 1)]


def test_a_new_state_evicts_the_oldest_turns_before_any_anchor() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=30, min_tokens=CHUNK)
    system = tokens(1, 10)
    states.keep(system, bytes(10), anchor=True)
    for turn in range(3):
        states.keep([*system, 50 + turn, 60, 61, 62], bytes(10), anchor=False)

    assert states.sizes() == ([10], [14, 14])
    assert states.held_bytes() <= 30


def test_anchors_alone_stay_within_the_budget_oldest_first() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=25, min_tokens=CHUNK)
    for head in (1, 2, 3):
        states.keep(tokens(head, 8), bytes(10), anchor=True)

    assert [prefix[0] for prefix in states._anchors] == [2, 3]
    assert states.held_bytes() == 20


def test_an_anchor_that_others_continue_outlives_them() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=30, min_tokens=CHUNK)
    shared = tokens(1, 8)
    states.keep(shared, bytes(10), anchor=True)
    for tail in (1, 2, 3):
        states.keep([*shared, tail, 7, 7, 7], bytes(10), anchor=True)

    assert [prefix[8:9] for prefix in states._anchors] == [(), (2,), (3,)]


def test_a_turn_just_kept_stays_with_its_anchor_when_the_cache_is_full_of_anchors() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=30, min_tokens=CHUNK)
    for head in (1, 2, 3):
        states.keep(tokens(head, 8), bytes(10), anchor=True)
    states.keep([*tokens(1, 8), 9, 9, 9, 9], bytes(10), anchor=False)
    after_the_first = [[prefix[0] for prefix in held] for held in (states._anchors, states._turns)]
    states.keep([*tokens(3, 8), 9, 9, 9, 9], bytes(10), anchor=False)

    assert after_the_first == [[1, 3], [1]]
    assert [prefix[0] for prefix in states._anchors] == [1, 3]
    assert [prefix[0] for prefix in states._turns] == [3]


def test_a_used_state_is_the_last_to_go() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=25, min_tokens=CHUNK)
    first, second = tokens(1, 8), tokens(2, 8)
    states.keep(first, bytes(10), anchor=True)
    states.keep(second, bytes(10), anchor=True)
    assert states.deepest([*first, 7]) is not None
    states.keep(tokens(3, 8), bytes(10), anchor=True)

    assert [prefix[0] for prefix in states._anchors] == [1, 3]


def test_the_deepest_held_prefix_wins_across_both_lists() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    system = tokens(1, 8)
    turn = [*system, 9, 9, 9, 9]
    states.keep(system, b"system", anchor=True)
    states.keep(turn, b"turn", anchor=False)

    assert states.deepest([*turn, 5]) == (tuple(turn), b"turn")
    assert states.deepest([*system, 5]) == (tuple(system), b"system")
    assert states.deepest(system) is None


def test_a_continued_conversation_replaces_its_shorter_turn() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    opening = tokens(1, 8)
    states.keep(opening, bytes(1), anchor=False)
    states.keep([*opening, 3, 3, 3, 3], bytes(1), anchor=False)

    assert states.sizes() == ([], [12])


def test_short_prefixes_are_not_kept_and_turns_go_on_unload() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    states.keep([1, 2], bytes(1), anchor=True)
    states.keep(tokens(1, 8), bytes(1), anchor=True)
    states.keep(tokens(1, 12), bytes(1), anchor=False)
    states.forget_turns()

    assert states.sizes() == ([8], [])


def test_an_unfinished_anchor_survives_the_unload_and_goes_when_a_state_continues_it() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    system = tokens(1, 12)
    states.keep(system[:6], bytes(1), anchor=True, unfinished=True)
    states.forget_turns()
    after_unload = states.sizes()
    states.keep(system[:9], bytes(1), anchor=True, unfinished=True)
    deeper = states.sizes()
    states.keep(system, bytes(1), anchor=True)

    assert (after_unload, deeper) == (([6], []), ([9], []))
    assert states.sizes() == ([12], [])


def test_a_finished_anchor_is_not_dropped_by_a_state_that_continues_it() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    system = tokens(1, 12)
    states.keep(system[:6], bytes(1), anchor=True, unfinished=True)
    states.keep(system[:6], bytes(1), anchor=True)
    states.keep(system, bytes(1), anchor=True)

    assert states.sizes() == ([6, 12], [])


def test_an_evicted_unfinished_anchor_leaves_no_trace() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=15, min_tokens=CHUNK)
    system = tokens(1, 12)
    states.keep(system[:6], bytes(10), anchor=True, unfinished=True)
    states.keep(tokens(2, 8), bytes(10), anchor=True)
    states.keep(system[:6], bytes(1), anchor=True)
    states.keep(system, bytes(1), anchor=True)

    assert sorted(states.sizes()[0]) == [6, 8, 12]


def test_the_stable_part_is_a_long_enough_common_start_inside_the_system() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    states.keep([1, 2, 3, 4, 5, 6, 7, 8], bytes(1), anchor=True)

    assert states.stable_prefix([1, 2, 3, 4, 5, 9, 9, 9, 9], shared=8) == 5
    assert states.stable_prefix([1, 2, 3, 9, 9, 9, 9, 9, 9], shared=8) == 0
    assert states.stable_prefix([1, 2, 3, 4, 5, 6, 7, 8, 9], shared=8) == 0


def test_a_read_cut_short_inside_the_system_is_no_stable_part() -> None:
    states: PromptStates[bytes] = PromptStates(budget_bytes=1000, min_tokens=CHUNK)
    states.keep([1, 2, 3, 4, 5], bytes(1), anchor=False)

    assert states.stable_prefix([1, 2, 3, 4, 5, 6, 7, 8, 9], shared=8) == 0
