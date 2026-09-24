from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import random
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from unidecode import unidecode

from eval import manifest as manifest_module
from eval.manifest import Manifest, Track, parse_lrc, plain_lines

WORKER_ROOT = Path(__file__).resolve().parent.parent
LRCLIB = "https://lrclib.net/api"
USER_AGENT = "scd-worker-eval/1.0 (https://github.com/soundcloud-desktop)"
DURATION_TOLERANCE_S = 4.0
MAX_TRACK_S = 450.0
MIN_LRC_LINES = {"positive": 12, "hard": 6}
MIN_POOL_LINES = 12
SYNTHETIC_PER_AUDIO = 10
SAME_SONG_LINE_SHARE = 0.2
CALIB_POSITIVES = 20
CALIB_MANUAL_NEGATIVES = 5
SEARCH_RESULTS = 6
MAX_DURATION_FILTERS = 6
SEED = 20260923
YTDLP_TIMEOUT_S = 300
UNSAFE = re.compile(r"[^a-z0-9]+")

log = logging.getLogger("eval.collect")

Lyrics = dict[str, object]


@dataclass(frozen=True)
class TextFrom:
    artist: str
    title: str
    language: str


@dataclass(frozen=True)
class Seed:
    artist: str
    title: str
    language: str | None
    kind: str = "positive"
    expected: str = "ok"
    tags: tuple[str, ...] = ()
    query: str | None = None
    note: str = ""
    text_from: TextFrom | None = None


@dataclass(frozen=True)
class Found:
    seed: Seed
    id: str
    audio_id: str
    duration_s: float
    plain: str
    synced: str | None
    text_file: str


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="eval.collect")
    parser.add_argument("--seeds", type=Path, default=WORKER_ROOT / "eval" / "seeds.json")
    parser.add_argument("--data", type=Path, default=Path(os.environ.get("EVAL_DATA_DIR", "")))
    parser.add_argument("--manifest", type=Path, default=WORKER_ROOT / "eval" / "manifest.json")
    parser.add_argument("--only", default="")
    parser.add_argument("--skip-download", action="store_true")
    args = parser.parse_args(argv)
    if not args.data:
        parser.error("--data (or EVAL_DATA_DIR) is required")
    logging.basicConfig(level=logging.INFO, stream=sys.stderr, format="%(levelname)s %(message)s")
    raw = json.loads(args.seeds.read_text(encoding="utf-8"))
    seeds = [parse_seed(item) for item in raw["seeds"]]
    pools = {code: list(texts) for code, texts in raw.get("text_pool_queries", {}).items()}
    wanted = {item for item in args.only.split(",") if item}
    collected = collect(
        [seed for seed in seeds if not wanted or slug(seed) in wanted],
        args.data,
        args.skip_download,
    )
    pool = text_pool(pools, collected, args.data)
    built = build_manifest(collected, pool, args.data)
    problems = manifest_module.check(built)
    manifest_module.save(built, args.manifest)
    log.info(
        "manifest: %d tracks (%d real audio), problems: %s",
        len(built.tracks),
        len({track.cluster for track in built.tracks}),
        problems or "none",
    )
    return 1 if problems else 0


def parse_seed(item: dict[str, object]) -> Seed:
    fields = dict(item)
    fields["tags"] = tuple(str(tag) for tag in item.get("tags", ()))
    source = item.get("text_from")
    fields["text_from"] = TextFrom(**source) if isinstance(source, dict) else None
    return Seed(**fields)


def collect(seeds: Sequence[Seed], data: Path, skip_download: bool) -> list[Found]:
    (data / "audio").mkdir(parents=True, exist_ok=True)
    (data / "texts").mkdir(parents=True, exist_ok=True)
    found: list[Found] = []
    for seed in seeds:
        try:
            found.append(collect_one(seed, data, skip_download))
        except CollectError as error:
            log.warning("skip %s: %s", slug(seed), error)
    return found


class CollectError(RuntimeError):
    pass


def collect_one(seed: Seed, data: Path, skip_download: bool) -> Found:
    audio_id = slug(seed)
    audio_dir = data / "audio"
    candidates = lrclib_search(seed.artist, seed.title)
    existing = next(audio_dir.glob(f"{audio_id}.*"), None)
    if existing is None:
        if skip_download:
            raise CollectError("audio missing and downloads are disabled")
        existing = download(seed, audio_id, audio_dir, synced_durations(candidates))
    actual = probe_duration(existing)
    if actual > MAX_TRACK_S:
        raise CollectError(f"track is {actual:.0f}s, longer than {MAX_TRACK_S:.0f}s")
    if seed.text_from is not None:
        return foreign_text(seed, audio_id, actual, data)
    lyrics = closest(candidates, actual)
    if seed.expected == "ok" and (lyrics is None or not lyrics.get("syncedLyrics")):
        raise CollectError(
            f"no synced lyrics on LRCLIB within {DURATION_TOLERANCE_S:.0f}s of {actual:.0f}s"
        )
    plain = str(lyrics.get("plainLyrics") or "") if lyrics else ""
    synced = str(lyrics.get("syncedLyrics") or "") if lyrics else None
    needed = MIN_LRC_LINES.get(seed.kind, MIN_LRC_LINES["positive"])
    if seed.expected == "ok" and len(parse_lrc(synced or "")) < needed:
        raise CollectError(f"reference LRC has fewer than {needed} lines")
    text_file = f"texts/{audio_id}.txt"
    (data / text_file).write_text(plain, encoding="utf-8")
    if synced:
        (data / "texts" / f"{audio_id}.lrc").write_text(synced, encoding="utf-8")
    return Found(seed, audio_id, audio_id, actual, plain, synced, text_file)


def foreign_text(seed: Seed, audio_id: str, duration_s: float, data: Path) -> Found:
    source = seed.text_from
    if source is None:
        raise CollectError("foreign text seed without text_from")
    entries = [
        item
        for item in lrclib_search(source.artist, source.title)
        if len(plain_lines(str(item.get("plainLyrics") or ""))) >= MIN_POOL_LINES
    ]
    if not entries:
        raise CollectError(f"no plain lyrics for {source.artist} - {source.title}")
    plain = str(
        max(entries, key=lambda item: len(str(item.get("plainLyrics") or "")))["plainLyrics"]
    )
    text_file = f"texts/{audio_id}.translation.txt"
    (data / text_file).write_text(plain, encoding="utf-8")
    return Found(seed, f"{audio_id}:translation", audio_id, duration_s, plain, None, text_file)


def lrclib_search(artist: str, title: str) -> list[Lyrics]:
    return lrclib_get("search", {"artist_name": artist, "track_name": title})


def synced_durations(candidates: Sequence[Lyrics]) -> list[float]:
    durations: list[float] = []
    for item in candidates:
        if not item.get("syncedLyrics"):
            continue
        duration = float(item.get("duration") or 0.0)
        if 0.0 < duration <= MAX_TRACK_S and all(abs(duration - d) > 1.0 for d in durations):
            durations.append(duration)
    return durations[:MAX_DURATION_FILTERS]


def closest(candidates: Sequence[Lyrics], duration_s: float) -> Lyrics | None:
    close = [
        item
        for item in candidates
        if abs(float(item.get("duration") or 0.0) - duration_s) <= DURATION_TOLERANCE_S
    ]
    synced = [item for item in close if item.get("syncedLyrics")] or close
    if not synced:
        return None
    return min(
        synced,
        key=lambda item: (
            not item.get("syncedLyrics"),
            abs(float(item.get("duration") or 0.0) - duration_s),
            -len(str(item.get("syncedLyrics") or "")),
        ),
    )


def lrclib_get(endpoint: str, params: dict[str, str]) -> list[Lyrics]:
    url = f"{LRCLIB}/{endpoint}?{urllib.parse.urlencode(params)}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.loads(response.read().decode("utf-8"))
                return payload if isinstance(payload, list) else [payload]
        except OSError as error:
            log.warning("lrclib %s attempt %d failed: %s", endpoint, attempt + 1, error)
            time.sleep(2.0 * (attempt + 1))
    raise CollectError("lrclib unreachable")


def download(seed: Seed, identifier: str, audio_dir: Path, durations: Sequence[float]) -> Path:
    query = seed.query or f"{seed.artist} - {seed.title}"
    template = str(audio_dir / f"{identifier}.%(ext)s")
    command = [
        "uvx",
        "yt-dlp",
        "--no-playlist",
        "--quiet",
        "--no-warnings",
        "-f",
        "bestaudio[ext=m4a]/bestaudio",
        "--max-downloads",
        "1",
        "-o",
        template,
    ]
    for duration_filter in duration_filters(durations):
        command += ["--match-filters", duration_filter]
    command.append(f"ytsearch{SEARCH_RESULTS}:{query}")
    try:
        result = subprocess.run(
            command, check=False, timeout=YTDLP_TIMEOUT_S, capture_output=True, text=True
        )
    except subprocess.TimeoutExpired as error:
        raise CollectError("yt-dlp timed out") from error
    path = next(audio_dir.glob(f"{identifier}.*"), None)
    if path is None:
        raise CollectError(f"yt-dlp downloaded nothing: {result.stderr.strip()[-200:]}")
    return path


def duration_filters(durations: Sequence[float]) -> list[str]:
    if not durations:
        return [f"duration<={MAX_TRACK_S:.0f}"]
    return [
        f"duration>={d - DURATION_TOLERANCE_S:.0f} & duration<={d + DURATION_TOLERANCE_S:.0f}"
        for d in durations
    ]


def probe_duration(path: Path) -> float:
    result = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    try:
        return float(result.stdout.strip())
    except ValueError as error:
        raise CollectError(f"ffprobe failed on {path.name}") from error


def text_pool(
    queries: dict[str, list[str]], collected: Sequence[Found], data: Path
) -> dict[str, list[tuple[str, str]]]:
    pool_dir = data / "pool"
    pool_dir.mkdir(parents=True, exist_ok=True)
    pool: dict[str, list[tuple[str, str]]] = {}
    own = {item.audio_id for item in collected}
    for code, artists in queries.items():
        entries: list[tuple[str, str]] = []
        for artist in artists:
            for item in lrclib_get("search", {"q": artist}):
                plain = str(item.get("plainLyrics") or "")
                if artist.lower() not in str(item.get("artistName") or "").lower():
                    continue
                if len(plain_lines(plain)) < MIN_POOL_LINES:
                    continue
                identifier = f"pool-{code}-{UNSAFE.sub('-', str(item['id']))}"
                if identifier in own:
                    continue
                path = pool_dir / f"{identifier}.txt"
                path.write_text(plain, encoding="utf-8")
                entries.append((identifier, str(path.relative_to(data))))
        pool[code] = dedupe(entries)
        log.info("text pool %s: %d texts", code, len(pool[code]))
    return pool


def dedupe(entries: list[tuple[str, str]]) -> list[tuple[str, str]]:
    seen: set[str] = set()
    unique: list[tuple[str, str]] = []
    for identifier, path in entries:
        if identifier not in seen:
            seen.add(identifier)
            unique.append((identifier, path))
    return unique


def build_manifest(
    collected: Sequence[Found], pool: dict[str, list[tuple[str, str]]], data: Path
) -> Manifest:
    rng = random.Random(SEED)
    positives = [item for item in collected if item.seed.expected == "ok"]
    audit = positives[0].id if positives else None
    calib_ids = {item.id for item in positives[1 : CALIB_POSITIVES + 1]}
    tracks: list[Track] = []
    calib_negatives = 0
    for item in collected:
        audio = audio_path(data, item.audio_id)
        if item.seed.expected == "ok":
            split = "calib" if item.id in calib_ids else "control"
        else:
            split = "calib" if calib_negatives < CALIB_MANUAL_NEGATIVES else "control"
            calib_negatives += 1
        text_file, source = item.text_file, "provider_plain"
        if item.seed.expected != "ok":
            source = "translation" if item.seed.text_from else "manual"
        if not plain_lines(item.plain):
            borrowed = pool.get(item.seed.language or "", [])
            if not borrowed:
                log.warning("skip %s: no text and no pool for %s", item.id, item.seed.language)
                continue
            text_file, source = borrowed[0][1], "pool"
        tracks.append(
            Track(
                id=item.id,
                kind=item.seed.kind,
                split=split,
                language=item.seed.language,
                audio=audio,
                input_text=text_file,
                input_source=source,
                reference_lrc=f"texts/{item.audio_id}.lrc" if item.synced else None,
                expected=item.seed.expected,
                cluster=item.audio_id,
                path=expected_path(item.seed.language),
                note=item.seed.note,
                tags=item.seed.tags,
            )
        )
    for index, item in enumerate(positives):
        language = item.seed.language or "none"
        candidates = [
            entry
            for entry in pool.get(language, [])
            if not same_song(item.plain, (data / entry[1]).read_text(encoding="utf-8"))
        ]
        rng.shuffle(candidates)
        for order, (identifier, path) in enumerate(candidates[:SYNTHETIC_PER_AUDIO]):
            tracks.append(
                Track(
                    id=f"{item.id}:{identifier}",
                    kind="synthetic",
                    split="calib" if index % 2 == 0 else "control",
                    language=item.seed.language,
                    audio=audio_path(data, item.audio_id),
                    input_text=path,
                    input_source="pool",
                    reference_lrc=None,
                    expected="lyrics_mismatch",
                    cluster=item.audio_id,
                    path=expected_path(item.seed.language),
                    note=f"foreign text {order}",
                    tags=("synthetic",),
                )
            )
    return Manifest(1, audit, tuple(tracks))


def same_song(own_text: str, other_text: str) -> bool:
    own = {normalized_line(line) for line in plain_lines(own_text)}
    other = {normalized_line(line) for line in plain_lines(other_text)}
    if not own:
        return False
    return len(own & other) / len(own) >= SAME_SONG_LINE_SHARE


def normalized_line(line: str) -> str:
    return UNSAFE.sub(" ", unidecode(line).lower()).strip()


def audio_path(data: Path, audio_id: str) -> str:
    return str(next(p for p in (data / "audio").glob(f"{audio_id}.*")).relative_to(data))


def expected_path(language: str | None) -> str:
    from worker.domain.language import ALIGNER_LANGUAGES, ASR_LANGUAGES

    if language in ALIGNER_LANGUAGES:
        return "qwen"
    if language in ASR_LANGUAGES:
        return "mms"
    return "global"


def slug(seed: Seed) -> str:
    base = UNSAFE.sub("-", unidecode(f"{seed.artist}-{seed.title}").lower()).strip("-")
    digest = hashlib.sha1(f"{seed.artist}|{seed.title}".encode()).hexdigest()[:6]
    return f"{base[:40]}-{digest}" if base else digest


if __name__ == "__main__":
    sys.exit(main())
