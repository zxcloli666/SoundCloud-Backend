from __future__ import annotations

import json
import re
from collections.abc import Sequence
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Literal

from eval import stats

MAX_FALSE_ACCEPT = 0.02
LRC_LINE = re.compile(r"^\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\](.*)$")

Split = Literal["calib", "control"]
Kind = Literal["positive", "queue", "hard", "negative", "synthetic"]


@dataclass(frozen=True)
class Track:
    id: str
    kind: Kind
    split: Split
    language: str | None
    audio: str
    input_text: str
    input_source: str
    reference_lrc: str | None
    expected: str
    cluster: str
    path: str = "qwen"
    note: str = ""
    tags: tuple[str, ...] = field(default_factory=tuple)

    @property
    def positive(self) -> bool:
        return self.expected == "ok"


@dataclass(frozen=True)
class Manifest:
    version: int
    audit_track: str | None
    tracks: tuple[Track, ...]

    def by_split(self, split: Split) -> list[Track]:
        return [track for track in self.tracks if track.split == split]

    def negatives(self) -> list[Track]:
        return [track for track in self.tracks if not track.positive]


def load(path: Path) -> Manifest:
    raw = json.loads(path.read_text(encoding="utf-8"))
    tracks = tuple(Track(**{**item, "tags": tuple(item.get("tags", ()))}) for item in raw["tracks"])
    return Manifest(int(raw["version"]), raw.get("audit_track"), tracks)


def save(manifest: Manifest, path: Path) -> None:
    payload = {
        "version": manifest.version,
        "audit_track": manifest.audit_track,
        "tracks": [{**asdict(track), "tags": list(track.tags)} for track in manifest.tracks],
    }
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")


def check(manifest: Manifest) -> list[str]:
    problems: list[str] = []
    ids = [track.id for track in manifest.tracks]
    if len(ids) != len(set(ids)):
        problems.append("duplicate track ids")
    negatives = manifest.negatives()
    if negatives:
        effective = negative_n_eff(negatives)
        needed = stats.n_min(MAX_FALSE_ACCEPT)
        if effective < needed:
            problems.append(f"negatives n_eff={effective:.0f} < n_min={needed}")
    for split in ("calib", "control"):
        if not manifest.by_split(split):
            problems.append(f"split {split} is empty")
    if manifest.audit_track and manifest.audit_track not in ids:
        problems.append("audit track is not in the manifest")
    return problems


def negative_n_eff(negatives: Sequence[Track], rho: float = stats.DEFAULT_RHO) -> float:
    clusters: dict[str, int] = {}
    for track in negatives:
        clusters[track.cluster] = clusters.get(track.cluster, 0) + 1
    mean_size = len(negatives) / max(1, len(clusters))
    return stats.n_eff(len(negatives), mean_size, rho)


def parse_lrc(text: str) -> list[tuple[float, str]]:
    lines: list[tuple[float, str]] = []
    for raw in text.splitlines():
        match = LRC_LINE.match(raw.strip())
        if match is None:
            continue
        minutes, seconds, fraction, body = match.groups()
        fraction_s = int(fraction.ljust(3, "0")[:3]) / 1000.0 if fraction else 0.0
        body = body.strip()
        if body:
            lines.append((int(minutes) * 60 + int(seconds) + fraction_s, body))
    return lines


def plain_lines(text: str) -> list[str]:
    return [line.strip() for line in text.splitlines() if line.strip()]
