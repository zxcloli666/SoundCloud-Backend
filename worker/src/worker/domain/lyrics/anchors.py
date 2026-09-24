from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Iterator, Mapping, Sequence
from dataclasses import dataclass
from itertools import pairwise, repeat
from typing import Protocol

from worker.domain.lyrics.regions import Region
from worker.domain.lyrics.tokens import SYLLABLE_KEYED, TokenizedLine, fold, similarity_key
from worker.settings import AnchorSettings

NEGATIVE_INFINITY = float("-inf")
MAX_FILL_RATIO = 2.0
NOISE_FLOOR = 0.1
SPELLING_FLOOR = 0.4
GRAM = 3
MAX_JOINED_REGIONS = 3
MAX_JOIN_GAP_S = 3.0
JOIN_PENALTY = 0.05

Table = dict[tuple[int, int], tuple[int, int]]


@dataclass(frozen=True)
class Assignment:
    region: int
    through: int
    lines: tuple[int, ...]


@dataclass(frozen=True)
class Anchoring:
    assignments: tuple[Assignment, ...]
    agreement: float
    similarities: Mapping[int, float]

    @property
    def placed(self) -> frozenset[int]:
        return frozenset(line for assignment in self.assignments for line in assignment.lines)


class Scorer(Protocol):
    def credits(
        self, keys: Sequence[str], lines: Sequence[TokenizedLine], first: int
    ) -> Iterator[float]: ...


@dataclass(frozen=True)
class Stretch:
    regions: int
    scorer: Scorer
    duration_s: float
    cost: float


def assign(
    drafts: Sequence[str | None],
    lines: Sequence[TokenizedLine],
    regions: Sequence[Region],
    settings: AnchorSettings,
) -> Anchoring:
    if not drafts or not lines:
        return Anchoring((), 0.0, {})
    keys = [line_key(line) for line in lines]
    seconds = [seconds_needed(line, settings) for line in lines]
    join_cost = settings.skip_region + JOIN_PENALTY
    spelling = stretches(drafts, lines, regions, MAX_JOINED_REGIONS, join_cost, DraftSpelling)
    placement = search(spelling, lines, keys, seconds, settings)
    if placement is None:
        return Anchoring((), 0.0, {})
    assignments = walk_back(placement, len(drafts), len(lines))
    words = stretches(drafts, lines, regions, 1, join_cost, DraftMatcher)
    identity = search(words, lines, keys, seconds, settings)
    matched = walk_back(identity, len(drafts), len(lines)) if identity is not None else ()
    return Anchoring(
        assignments,
        agreement(pair_similarities(matched, drafts, lines, keys), lines, unheard(matched, drafts)),
        pair_similarities(assignments, drafts, lines, keys),
    )


def line_key(line: TokenizedLine) -> str:
    if line.language in SYLLABLE_KEYED:
        return similarity_key(" ".join(line.tokens), line.language)
    if line.cjk:
        return " ".join(fold(token) for token in line.romanized if token)
    return " ".join(fold(token) for token in line.tokens)


class DraftMatcher:
    def __init__(self, draft: str, lines: Sequence[TokenizedLine]) -> None:
        languages = {line.language for line in lines}
        words = {language: similarity_key(draft, language).split() for language in languages}
        self._words = {language: set(items) for language, items in words.items()}
        self._bigrams = {language: bigrams(items) for language, items in words.items()}

    def single(self, key: str, language: str | None) -> float:
        words = key.split()
        if not words or not self._words[language]:
            return 0.0
        if len(words) == 1:
            return 1.0 if words[0] in self._words[language] else 0.0
        pairs = bigrams(words)
        return len(pairs & self._bigrams[language]) / len(pairs)

    def credits(
        self, keys: Sequence[str], lines: Sequence[TokenizedLine], first: int
    ) -> Iterator[float]:
        for index in range(first, len(lines)):
            yield self.single(keys[index], lines[index].language) - NOISE_FLOOR


class DraftSpelling:
    def __init__(self, draft: str, lines: Sequence[TokenizedLine]) -> None:
        languages = {line.language for line in lines}
        self._grams = {
            language: letter_grams(similarity_key(draft, language)) for language in languages
        }

    def credits(
        self, keys: Sequence[str], lines: Sequence[TokenizedLine], first: int
    ) -> Iterator[float]:
        unused: dict[str | None, Counter[str]] = {}
        for index in range(first, len(lines)):
            language = lines[index].language
            if language not in unused:
                unused[language] = Counter(self._grams[language])
            yield consume(letter_grams(keys[index]), unused[language]) - SPELLING_FLOOR


class Unheard:
    def credits(
        self, keys: Sequence[str], lines: Sequence[TokenizedLine], first: int
    ) -> Iterator[float]:
        return repeat(-NOISE_FLOOR, len(lines) - first)


def stretches(
    drafts: Sequence[str | None],
    lines: Sequence[TokenizedLine],
    regions: Sequence[Region],
    longest: int,
    join_cost: float,
    scorer: Callable[[str, Sequence[TokenizedLine]], Scorer],
) -> list[list[Stretch]]:
    options: list[list[Stretch]] = []
    for first in range(len(drafts)):
        here: list[Stretch] = []
        for last in range(first, min(len(drafts), first + longest)):
            if last > first and regions[last].start_s - regions[last - 1].end_s > MAX_JOIN_GAP_S:
                break
            text = joined(drafts[first : last + 1])
            here.append(
                Stretch(
                    last - first + 1,
                    scorer(text, lines) if text else Unheard(),
                    regions[last].end_s - regions[first].start_s,
                    join_cost * (last - first),
                )
            )
        options.append(here)
    return options


def joined(drafts: Sequence[str | None]) -> str:
    return " ".join(draft for draft in drafts if draft)


def consume(wanted: Counter[str], unused: Counter[str]) -> float:
    total = sum(wanted.values())
    if total == 0:
        return 0.0
    found = {gram: min(count, unused[gram]) for gram, count in wanted.items()}
    unused.subtract(found)
    return sum(found.values()) / total


def letter_grams(key: str) -> Counter[str]:
    padded = f" {key} "
    return Counter(padded[start : start + GRAM] for start in range(len(padded) - GRAM + 1))


def search(
    options: Sequence[Sequence[Stretch]],
    lines: Sequence[TokenizedLine],
    keys: Sequence[str],
    seconds: Sequence[float],
    settings: AnchorSettings,
) -> Table | None:
    regions, count = len(options), len(lines)
    table = [[NEGATIVE_INFINITY] * (count + 1) for _ in range(regions + 1)]
    choice: Table = {}
    table[0][0] = 0.0
    for region in range(regions + 1):
        for line in range(count + 1):
            here = table[region][line]
            if here == NEGATIVE_INFINITY:
                continue
            if region < regions:
                relax(table, choice, region + 1, line, here - settings.skip_region, (region, line))
            if line < count:
                relax(table, choice, region, line + 1, here - settings.skip_line, (region, line))
            if region == regions or line == count:
                continue
            for stretch in options[region]:
                for taken, gain in group_gains(stretch, lines, keys, seconds, line, settings):
                    target = (region + stretch.regions, line + taken)
                    relax(table, choice, *target, here + gain - stretch.cost, (region, line))
    if table[regions][count] == NEGATIVE_INFINITY:
        return None
    return choice


def group_gains(
    stretch: Stretch,
    lines: Sequence[TokenizedLine],
    keys: Sequence[str],
    seconds: Sequence[float],
    first: int,
    settings: AnchorSettings,
) -> list[tuple[int, float]]:
    duration = max(stretch.duration_s, 0.0)
    limit = max(duration * MAX_FILL_RATIO, seconds[first])
    credits = stretch.scorer.credits(keys, lines, first)
    needed = 0.0
    gain = 0.0
    gains: list[tuple[int, float]] = []
    for taken, credit in zip(range(1, len(lines) - first + 1), credits, strict=False):
        needed += seconds[first + taken - 1]
        if needed > limit and taken > 1:
            break
        gain += credit
        gains.append((taken, gain - overflow(needed, duration, settings.overflow)))
    return gains


def seconds_needed(line: TokenizedLine, settings: AnchorSettings) -> float:
    rate = settings.cjk_chars_per_s if line.cjk else settings.words_per_s
    return line.units / rate


def overflow(needed_s: float, duration_s: float, penalty: float) -> float:
    return penalty * max(0.0, needed_s - duration_s) / max(1.0, duration_s)


def relax(
    table: list[list[float]],
    choice: Table,
    region: int,
    line: int,
    value: float,
    came_from: tuple[int, int],
) -> None:
    if value > table[region][line]:
        table[region][line] = value
        choice[(region, line)] = came_from


def walk_back(choice: Table, regions: int, count: int) -> tuple[Assignment, ...]:
    assigned: dict[int, Assignment] = {}
    region, line = regions, count
    while (region, line) in choice:
        previous_region, previous_line = choice[(region, line)]
        if previous_region < region and previous_line < line:
            assigned[previous_region] = Assignment(
                previous_region, region - 1, tuple(range(previous_line, line))
            )
        elif previous_region < region:
            assigned[previous_region] = Assignment(previous_region, previous_region, ())
        region, line = previous_region, previous_line
    return tuple(assigned[index] for index in sorted(assigned))


def pair_similarities(
    assignments: Sequence[Assignment],
    drafts: Sequence[str | None],
    lines: Sequence[TokenizedLine],
    keys: Sequence[str],
) -> dict[int, float]:
    similarities: dict[int, float] = {}
    for assignment in assignments:
        text = joined(drafts[assignment.region : assignment.through + 1])
        if not text or not assignment.lines:
            continue
        matcher = DraftMatcher(text, lines)
        for line in assignment.lines:
            similarities[line] = matcher.single(keys[line], lines[line].language)
    return similarities


def unheard(assignments: Sequence[Assignment], drafts: Sequence[str | None]) -> frozenset[int]:
    return frozenset(
        line
        for assignment in assignments
        if not joined(drafts[assignment.region : assignment.through + 1])
        for line in assignment.lines
    )


def agreement(
    similarities: Mapping[int, float], lines: Sequence[TokenizedLine], unheard: frozenset[int]
) -> float:
    weights = {line.ordinal: max(1, line.units) for line in lines if line.ordinal not in unheard}
    total = sum(weights.values())
    if total == 0:
        return 0.0
    return sum(value * weights[line] for line, value in similarities.items()) / total


def bigrams(words: Sequence[str]) -> set[tuple[str, str]]:
    return set(pairwise(words))
