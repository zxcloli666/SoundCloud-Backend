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
