from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, replace

from worker.domain.lyrics.placement import Placement, Word
from worker.domain.lyrics.text import Line, Script
from worker.domain.lyrics.tokens import TokenizedLine

LEADING_STEP_S = 0.01


@dataclass(frozen=True)
class Stamped:
    entry: Line
    ordinal: int | None
    start_s: float
    end_s: float
    unplaced: bool
    interpolated: bool
    words: tuple[Word, ...]

    @property
    def placed(self) -> bool:
        return self.ordinal is not None and not self.unplaced


@dataclass(frozen=True)
class Rendered:
    synced_lrc: str
    words: list[dict[str, object]]


def render(
    script: Script, lines: Sequence[TokenizedLine], placement: Placement, *, pause_s: float
) -> Rendered:
    stamped = stamp(script, placement)
    return Rendered(synced_lrc(stamped, pause_s), words_payload(stamped, lines))


def stamp(script: Script, placement: Placement) -> list[Stamped]:
    stamped: list[Stamped] = []
    ordinal = 0
    last_start = 0.0
    last_end: float | None = None
    for entry in script.entries:
        if not entry.sung:
            start = last_end if last_end is not None else last_start
            stamped.append(Stamped(entry, None, start, start, False, False, ()))
            continue
        timing = placement.get(ordinal)
        if timing is None:
            start = last_end if last_end is not None else last_start
            stamped.append(Stamped(entry, ordinal, start, start, True, False, ()))
        else:
            start = max(timing.start_s, last_start)
            end = max(timing.end_s, start)
            interpolated = timing.source == "interpolated"
            stamped.append(Stamped(entry, ordinal, start, end, False, interpolated, timing.words))
            last_start, last_end = start, end
        ordinal += 1
    return monotone(backdate_leading(stamped))


def synced_lrc(stamped: Sequence[Stamped], pause_s: float) -> str:
    rendered: list[str] = []
    previous: Stamped | None = None
    marker_between = False
    for current in stamped:
        if not current.entry.sung:
            rendered.append(timestamp(current.start_s))
            marker_between = True
            continue
        if previous is not None and not marker_between and pause(previous, current, pause_s):
            rendered.append(timestamp(previous.end_s))
        rendered.append(f"{timestamp(current.start_s)}{current.entry.text}")
        previous = current
        marker_between = False
    return "\n".join(rendered)


def words_payload(
    stamped: Sequence[Stamped], lines: Sequence[TokenizedLine]
) -> list[dict[str, object]]:
    payload: list[dict[str, object]] = []
    for item in stamped:
        if item.ordinal is None:
            continue
        if item.unplaced:
            payload.extend(
                word_entry(item.ordinal, token, item.start_s, item.start_s, False, True)
                for token in lines[item.ordinal].tokens
            )
            continue
        payload.extend(
            word_entry(
                item.ordinal,
                word.text,
                max(word.start_s, item.start_s),
                max(word.end_s, item.start_s),
                item.interpolated,
                False,
            )
            for word in item.words
        )
    return payload


def timestamp(seconds: float) -> str:
    centis = max(0, round(seconds * 100))
    minutes, rest = divmod(centis, 6000)
    return f"[{minutes:02d}:{rest // 100:02d}.{rest % 100:02d}]"


def backdate_leading(stamped: list[Stamped]) -> list[Stamped]:
    first_placed = next((index for index, item in enumerate(stamped) if item.placed), None)
    if first_placed is None:
        return stamped
    anchor = stamped[first_placed].start_s
    for index in range(first_placed):
        start = max(0.0, anchor - LEADING_STEP_S * (first_placed - index))
        stamped[index] = replace(stamped[index], start_s=start, end_s=start)
    return stamped


def monotone(stamped: list[Stamped]) -> list[Stamped]:
    ceiling: float | None = None
    for index in range(len(stamped) - 1, -1, -1):
        item = stamped[index]
        if item.placed:
            ceiling = item.start_s
        elif ceiling is not None and item.start_s > ceiling:
            stamped[index] = replace(item, start_s=ceiling, end_s=ceiling)
    floor = 0.0
    for index, item in enumerate(stamped):
        if item.start_s < floor:
            stamped[index] = replace(item, start_s=floor, end_s=max(item.end_s, floor))
        floor = stamped[index].start_s
    return stamped


def pause(previous: Stamped, current: Stamped, pause_s: float) -> bool:
    if previous.unplaced or current.unplaced:
        return False
    return current.start_s - previous.end_s >= pause_s


def word_entry(
    line: int, text: str, start_s: float, end_s: float, interpolated: bool, unplaced: bool
) -> dict[str, object]:
    return {
        "line": line,
        "text": text,
        "start_ms": max(0, round(start_s * 1000)),
        "end_ms": max(0, round(end_s * 1000)),
        "interpolated": interpolated,
        "unplaced": unplaced,
    }
