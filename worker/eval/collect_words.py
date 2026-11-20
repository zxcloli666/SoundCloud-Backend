from __future__ import annotations

import json
import logging
from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, replace
from pathlib import Path

from unidecode import unidecode

from eval import lrclib, lyricsfile
from eval import manifest as manifest_module
from eval.collect import (
    DURATION_TOLERANCE_S,
    MIN_LRC_LINES,
    UNSAFE,
    CollectError,
    Seed,
    audio_path,
    download,
    expected_path,
    probe_duration,
)
from eval.lyricsfile import Lyricsfile
from eval.manifest import Manifest, Track, plain_lines

PER_QUERY = 2
MIN_WORD_LINES = 8
MIN_WORDS = 60
DUPLICATE_DURATION_S = 2.0

log = logging.getLogger("eval.collect")


@dataclass(frozen=True)
class Query:
    artist: str
    title: str
    language: str


def run(seeds_path: Path, data: Path, manifest_path: Path, skip_download: bool) -> int:
    raw = json.loads(seeds_path.read_text(encoding="utf-8"))
    queries = [Query(**item) for item in raw["seeds"]]
    (data / "audio").mkdir(parents=True, exist_ok=True)
    (data / "texts").mkdir(parents=True, exist_ok=True)
    tracks: list[Track] = []
    skips: Counter[str] = Counter()
    taken: dict[str, list[float]] = {}
    for index, query in enumerate(queries, start=1):
        log.info("query %d/%d: %s - %s (%s)", index, len(queries), *query_fields(query))
        for record in word_synced(query, taken):
            try:
                track = collect_record(record, query, data, skip_download)
            except (CollectError, lrclib.LrclibError, lyricsfile.LyricsfileError) as error:
                reason = str(error)
                log.warning("skip %s: %s", record["id"], reason)
                skips[reason.split(":")[0]] += 1
                continue
            tracks.append(track)
            log.info("track %s: %s", track.id, track.note)
    built = build_manifest(tracks)
    problems = manifest_module.check(built)
    manifest_module.save(built, manifest_path)
    log.info(
        "total: %d tracks, %d skipped %s, problems: %s",
        len(tracks),
        sum(skips.values()),
        dict(skips),
        problems or "none",
    )
    return 1 if problems else 0


def query_fields(query: Query) -> tuple[str, str, str]:
    return query.artist, query.title, query.language


def word_synced(query: Query, taken: dict[str, list[float]]) -> list[Mapping[str, object]]:
    chosen: list[Mapping[str, object]] = []
    for record in search(f"{query.artist} {query.title}"):
        if not record.get("hasWordSync") or record.get("instrumental"):
            continue
        title = title_key(str(record.get("trackName")))
        duration = record.get("duration")
        if is_duplicate(taken.get(title, []), duration):
            log.info("skip %s: duplicate of %s", record.get("id"), title)
            continue
        if len(chosen) == PER_QUERY:
            log.info("skip %s: more than %d per query", record.get("id"), PER_QUERY)
            continue
        if isinstance(duration, int | float):
            taken.setdefault(title, []).append(float(duration))
        chosen.append(record)
    if not chosen:
        log.warning("skip query: no word-synced records")
    return chosen


def search(text: str) -> list[Mapping[str, object]]:
    try:
        payload = lrclib.get_json("v2/search", {"q": text})
    except lrclib.LrclibError as error:
        log.warning("skip query %s: %s", text, error)
        return []
    if not isinstance(payload, list):
        log.warning("skip query %s: search did not return a list", text)
        return []
    return [item for item in payload if isinstance(item, dict)]


def title_key(title: str) -> str:
    return UNSAFE.sub(" ", unidecode(title).lower()).strip()


def is_duplicate(durations: Sequence[float], duration: object) -> bool:
    if not isinstance(duration, int | float):
        return False
    return any(abs(duration - known) <= DUPLICATE_DURATION_S for known in durations)


def collect_record(
    record: Mapping[str, object], query: Query, data: Path, skip_download: bool
) -> Track:
    record_id = int(str(record["id"]))
    full = lrclib.get_json(f"get/{record_id}")
    if not isinstance(full, dict) or not full.get("lyricsfile"):
        raise CollectError("record has no lyricsfile")
    parsed = lyricsfile.parse(str(full["lyricsfile"]))
    words = parsed.words
    word_lines = sum(1 for line in parsed.lines if line.words)
    if len(parsed.lines) < MIN_LRC_LINES["positive"]:
        raise CollectError(f"fewer than {MIN_LRC_LINES['positive']} lines")
    if word_lines < MIN_WORD_LINES or len(words) < MIN_WORDS:
        raise CollectError(f"too few words: {word_lines} lines, {len(words)} words")
    plain = parsed.plain.strip() or str(full.get("plainLyrics") or "")
    if not plain_lines(plain):
        raise CollectError("no plain text")
    expected_s = expected_duration_s(parsed, full)
    audio_id = f"w{record_id}"
    audio = next((data / "audio").glob(f"{audio_id}.*"), None)
    if audio is None:
        if skip_download:
            raise CollectError("audio missing and downloads are disabled")
        seed = Seed(str(full["artistName"]), str(full["trackName"]), query.language)
        audio = download(seed, audio_id, data / "audio", [expected_s])
    actual = probe_duration(audio)
    if abs(actual - expected_s) > DURATION_TOLERANCE_S:
        raise CollectError(f"duration mismatch: audio {actual:.0f}s, lyricsfile {expected_s:.0f}s")
    write_texts(data, audio_id, plain, parsed)
    language = (parsed.language or query.language).lower().split("-")[0]
    return Track(
        id=audio_id,
        kind="positive",
        split="control",
        language=language,
        audio=audio_path(data, audio_id),
        input_text=f"texts/{audio_id}.txt",
        input_source="provider_plain",
        reference_lrc=f"texts/{audio_id}.lrc",
        expected="ok",
        cluster=title_key(str(full["trackName"])) or audio_id,
        path=expected_path(language),
        note=(
            f"{full['artistName']} - {full['trackName']} {language}"
            f" lines={len(parsed.lines)} words={len(words)} dropped={parsed.dropped_word_lines}"
        ),
        reference_words=f"texts/{audio_id}.words.json",
    )


def expected_duration_s(parsed: Lyricsfile, full: Mapping[str, object]) -> float:
    if parsed.duration_ms:
        return parsed.duration_ms / 1000.0
    duration = full.get("duration")
    if isinstance(duration, int | float) and duration > 0:
        return float(duration)
    raise CollectError("record has no duration")


def write_texts(data: Path, audio_id: str, plain: str, parsed: Lyricsfile) -> None:
    texts = data / "texts"
    (texts / f"{audio_id}.txt").write_text(plain, encoding="utf-8")
    (texts / f"{audio_id}.lrc").write_text(lyricsfile.to_lrc(parsed), encoding="utf-8")
    payload = lyricsfile.words_to_json(parsed.words)
    (texts / f"{audio_id}.words.json").write_text(
        json.dumps(payload, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def build_manifest(tracks: Sequence[Track]) -> Manifest:
    split = [
        replace(track, split="calib" if index % 2 == 0 else "control")
        for index, track in enumerate(tracks)
    ]
    return Manifest(1, None, tuple(split))
