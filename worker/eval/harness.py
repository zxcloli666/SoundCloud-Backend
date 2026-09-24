from __future__ import annotations

import argparse
import asyncio
import dataclasses
import json
import logging
import os
import shutil
import sys
import time
from collections import defaultdict
from collections.abc import Callable, Mapping, Sequence
from dataclasses import asdict, dataclass, replace
from difflib import SequenceMatcher
from functools import cached_property
from pathlib import Path
from typing import Literal

from eval import reference as reference_tools
from eval import stats
from eval.collect import CollectError, probe_duration
from eval.engines import LocalEngines
from eval.manifest import Manifest, Track, load, parse_lrc, plain_lines
from eval.reference import ReferenceLine
from worker import settings as settings_module
from worker.domain.deadline import Deadline
from worker.domain.lyrics.tokens import is_cjk, tempo_units, tokenize
from worker.domain.lyrics.transcribe import TranscribeLane
from worker.domain.workspace import Workspace
from worker.observability.counters import Counters
from worker.settings import Settings

WORKER_ROOT = Path(__file__).resolve().parent.parent
LANE_LOGGER = "worker.domain.lyrics.transcribe"
METRIC_KEYS = (
    "aligned_share",
    "placed_share",
    "interpolated_share",
    "inside_share",
    "collapsed_share",
    "out_of_order_share",
    "rate_outliers",
    "anchor_agreement",
    "language_agreement",
    "aligner_score",
    "separated",
)
ENGINE_PATHS = ("qwen", "mms", "gapfill", "global")
DEADLINE_S = 900.0
THRESHOLDS_S = (0.3, 0.5, 1.0)
MAX_FALSE_ACCEPT = 0.02
STRATEGIES = ("anchored", "global_ctc", "no_draft")
CUDA_ALLOCATOR = "expandable_segments:True"
MAX_UNITS_PER_S = 10.0
MAX_CJK_UNITS_PER_S = 20.0

Flavor = Literal["usable", "fitted", "raw"]

log = logging.getLogger("eval")


@dataclass(frozen=True)
class TrackResult:
    id: str
    kind: str
    split: str
    cluster: str
    language: str | None
    input_source: str
    expected: str
    status: str
    reason: str | None
    confidence: float | None
    metrics: Mapping[str, object]
    paths: Mapping[str, int]
    line_times: Mapping[int, tuple[float, float]]
    reference_lines: tuple[ReferenceLine, ...]
    line_features: Mapping[int, Sequence[float]]
    coverage: float | None
    audio_s: float
    gpu_s: Mapping[str, float]
    wall_s: float
    lines_total: int | None
    lines_unplaced: int | None

    @property
    def accepted(self) -> bool:
        return self.status == "ok"

    @cached_property
    def reference(self) -> reference_tools.ReferenceCheck:
        return reference_tools.check(self.line_times, self.reference_lines)

    @property
    def line_deltas(self) -> dict[int, float]:
        return self.reference.deltas(self.line_times)

    @property
    def deltas_s(self) -> tuple[float, ...]:
        return tuple(self.line_deltas.values())

    @property
    def raw_deltas_s(self) -> tuple[float, ...]:
        return tuple(produced - expected for produced, expected in self.line_times.values())

    @property
    def path(self) -> str:
        if not self.paths:
            return "none"
        return max(self.paths, key=lambda name: (self.paths[name], name))


class LocalAudioSource:
    async def fetch(self, url: str, into: Path, deadline: Deadline) -> int:
        source = Path(url.removeprefix("file://"))
        shutil.copyfile(source, into)
        return source.stat().st_size


class MetricCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.last: dict[str, object] = {}
        self.line_features: dict[int, list[float]] = {}

    def reset(self) -> None:
        self.last = {}
        self.line_features = {}

    def emit(self, record: logging.LogRecord) -> None:
        if record.getMessage() == "lyrics synced":
            self.last = {key: getattr(record, key) for key in METRIC_KEYS if hasattr(record, key)}
        if record.getMessage() == "line features":
            self.line_features = {
                int(line): list(row) for line, row in getattr(record, "line_features", {}).items()
            }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="eval.harness")
    parser.add_argument("--manifest", type=Path, default=WORKER_ROOT / "eval" / "manifest.json")
    parser.add_argument("--data", type=Path, default=Path(os.environ.get("EVAL_DATA_DIR", "")))
    parser.add_argument("--out", type=Path, default=None)
    parser.add_argument("--strategy", choices=STRATEGIES, default="anchored")
    parser.add_argument("--device", default="cuda")
    parser.add_argument("--ids", default="")
    parser.add_argument("--kinds", default="")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--config", type=Path, default=WORKER_ROOT / "config")
    args = parser.parse_args(argv)
    if not args.data or not args.data.is_dir():
        parser.error("--data (or EVAL_DATA_DIR) must point to the eval audio directory")
    configure_logging()
    os.environ.setdefault("PYTORCH_CUDA_ALLOC_CONF", CUDA_ALLOCATOR)
    settings = settings_module.load(args.config, base_environment())
    manifest = load(args.manifest)
    tracks = select(manifest, args.ids, args.kinds, args.limit)
    version = settings_module.sync_version(settings)
    out = args.out or WORKER_ROOT / "eval" / "results" / f"{version}.{args.strategy}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    journal = Journal(out.with_suffix(".partial.jsonl"))
    report = asyncio.run(
        run(settings, manifest, tracks, args.data, args.strategy, args.device, journal)
    )
    out.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    journal.remove()
    print(json.dumps(report["summary"], ensure_ascii=False, indent=1))
    print(f"written {out}")
    return 0


def configure_logging() -> None:
    console = logging.StreamHandler(sys.stderr)
    console.setLevel(logging.INFO)
    console.setFormatter(logging.Formatter("%(levelname)s %(message)s"))
    logging.basicConfig(level=logging.INFO, handlers=[console])
    logging.getLogger(LANE_LOGGER).setLevel(logging.DEBUG)


class Journal:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.done: dict[str, TrackResult] = {}
        self.crashed: set[str] = set()
        if not path.is_file():
            return
        started: set[str] = set()
        for line in path.read_text(encoding="utf-8").splitlines():
            entry = json.loads(line)
            if entry.get("started"):
                started.add(str(entry["id"]))
            else:
                self.done[str(entry["id"])] = result_from_json(entry)
        self.crashed = started - set(self.done)
        log.info(
            "resuming %s: %d done, %d crashed the previous process",
            path.name,
            len(self.done),
            len(self.crashed),
        )

    def starting(self, track_id: str) -> None:
        self._append({"id": track_id, "started": True})

    def finished(self, result: TrackResult) -> None:
        self.done[result.id] = result
        self._append(result_to_json(result))

    def remove(self) -> None:
        if self.path.is_file():
            self.path.unlink()

    def _append(self, entry: Mapping[str, object]) -> None:
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")


async def run(
    settings: Settings,
    manifest: Manifest,
    tracks: Sequence[Track],
    data: Path,
    strategy: str,
    device: str,
    journal: Journal,
) -> dict[str, object]:
    sync = settings.sync
    if strategy == "global_ctc":
        sync = replace(sync, strategy="global_ctc")
    engines = LocalEngines(settings, device, drafts=strategy != "no_draft")
    engines.load()
    capture = MetricCapture()
    logging.getLogger(LANE_LOGGER).addHandler(capture)
    counters = Counters()
    lane = TranscribeLane(
        engines,
        LocalAudioSource(),
        Workspace(data / "work", counters),
        settings.audio,
        sync,
        settings_module.sync_version(settings),
        counters,
    )
    results: list[TrackResult] = []
    try:
        for index, track in enumerate(tracks):
            if track.id in journal.done:
                results.append(journal.done[track.id])
                continue
            if track.id in journal.crashed:
                result = crashed_result(track, data)
            else:
                journal.starting(track.id)
                result = await evaluate(lane, engines, counters, capture, track, data)
            journal.finished(result)
            results.append(result)
            log.info(
                "%d/%d %s %s %s conf=%s path=%s acc@0.5=%.2f",
                index + 1,
                len(tracks),
                track.id,
                result.status,
                result.reason or "",
                result.confidence,
                result.path,
                stats.accuracy_at(result.deltas_s, 0.5),
            )
    finally:
        logging.getLogger(LANE_LOGGER).removeHandler(capture)
        engines.unload()
    return {
        "sync_version": settings_module.sync_version(settings),
        "strategy": strategy,
        "device": device,
        "manifest_version": manifest.version,
        "audit_track": manifest.audit_track,
        "summary": summarize(results, manifest, peak_vram_mib()),
        "engine_calls": dict(engines.calls),
        "engine_memo_hits": dict(engines.memo_hits),
        "tracks": [track_payload(result) for result in results],
    }


async def evaluate(
    lane: TranscribeLane,
    engines: LocalEngines,
    counters: Counters,
    capture: MetricCapture,
    track: Track,
    data: Path,
) -> TrackResult:
    audio = data / track.audio
    text = (data / track.input_text).read_text(encoding="utf-8")
    reference = (
        (data / track.reference_lrc).read_text(encoding="utf-8") if track.reference_lrc else None
    )
    request = {
        "sc_track_id": "1",
        "upload_generation": 1,
        "attempt": 1,
        "audio_url": f"file://{audio}",
        "reference_text": text,
        "reference_lines_total": len(plain_lines(text)),
        "language": track.language,
        "mode": "align",
    }
    before_paths = {
        name: counters.value("align_engine_total", engine=name) for name in ENGINE_PATHS
    }
    before_gpu = dict(engines.gpu_seconds)
    capture.reset()
    started = time.perf_counter()
    outcome = await lane.process(request, Deadline.after(DEADLINE_S))
    wall = time.perf_counter() - started
    fields = dict(outcome.fields)
    times, coverage = compare(str(fields.get("synced_lrc", "")), reference)
    return TrackResult(
        id=track.id,
        kind=track.kind,
        split=track.split,
        cluster=track.cluster,
        language=track.language,
        input_source=track.input_source,
        expected=track.expected,
        status=outcome.status.value,
        reason=outcome.reason.value if outcome.reason else None,
        confidence=float(fields["confidence"]) if "confidence" in fields else None,
        metrics=dict(capture.last),
        paths={
            name: counters.value("align_engine_total", engine=name) - before_paths[name]
            for name in ENGINE_PATHS
        },
        line_times=times,
        reference_lines=reference_lines(reference, track.language),
        line_features=dict(capture.line_features),
        coverage=coverage,
        audio_s=audio_duration_s(audio),
        gpu_s={
            slot: engines.gpu_seconds.get(slot, 0.0) - before_gpu.get(slot, 0.0)
            for slot in engines.gpu_seconds
        },
        wall_s=wall,
        lines_total=int(fields["lines_total"]) if "lines_total" in fields else None,
        lines_unplaced=int(fields["lines_unplaced"]) if "lines_unplaced" in fields else None,
    )


def crashed_result(track: Track, data: Path) -> TrackResult:
    log.warning("%s crashed the previous process, recorded as failed", track.id)
    return TrackResult(
        id=track.id,
        kind=track.kind,
        split=track.split,
        cluster=track.cluster,
        language=track.language,
        input_source=track.input_source,
        expected=track.expected,
        status="failed",
        reason="process_crashed",
        confidence=None,
        metrics={},
        paths={},
        line_times={},
        reference_lines=(),
        line_features={},
        coverage=None,
        audio_s=audio_duration_s(data / track.audio),
        gpu_s={},
        wall_s=0.0,
        lines_total=None,
        lines_unplaced=None,
    )


def result_to_json(result: TrackResult) -> dict[str, object]:
    payload = asdict(result)
    payload["line_times"] = {str(line): list(pair) for line, pair in result.line_times.items()}
    payload["line_features"] = {str(line): list(row) for line, row in result.line_features.items()}
    return payload


def result_from_json(entry: Mapping[str, object]) -> TrackResult:
    values = {field.name: entry[field.name] for field in dataclasses.fields(TrackResult)}
    values["line_times"] = {
        int(line): (float(pair[0]), float(pair[1]))
        for line, pair in dict(values["line_times"]).items()
    }
    values["reference_lines"] = tuple(
        ReferenceLine(float(start), float(shortest))
        for start, shortest in list(values["reference_lines"])
    )
    values["line_features"] = {
        int(line): [float(v) for v in row] for line, row in dict(values["line_features"]).items()
    }
    return TrackResult(**values)


def reference_lines(reference: str | None, language: str | None) -> tuple[ReferenceLine, ...]:
    limit = MAX_CJK_UNITS_PER_S if is_cjk(language) else MAX_UNITS_PER_S
    return tuple(
        ReferenceLine(start, tempo_units(tokenize(body, language), language) / limit)
        for start, body in parse_lrc(reference or "")
    )


def compare(
    synced_lrc: str, reference: str | None
) -> tuple[dict[int, tuple[float, float]], float | None]:
    if not reference or not synced_lrc:
        return {}, None
    produced = parse_lrc(synced_lrc)
    expected = parse_lrc(reference)
    if not expected:
        return {}, None
    matcher = SequenceMatcher(
        None,
        [key(text) for _, text in produced],
        [key(text) for _, text in expected],
        autojunk=False,
    )
    times: dict[int, tuple[float, float]] = {}
    for tag, i1, i2, j1, _ in matcher.get_opcodes():
        if tag != "equal":
            continue
        for offset in range(i2 - i1):
            times[i1 + offset] = (produced[i1 + offset][0], expected[j1 + offset][0])
    return times, len(times) / len(expected)


def key(text: str) -> str:
    return " ".join(text.lower().split())


def summarize(
    results: Sequence[TrackResult], manifest: Manifest, peak_vram: Mapping[str, float]
) -> dict[str, object]:
    positives = [result for result in results if result.expected == "ok"]
    negatives = [result for result in results if result.expected != "ok"]
    summary: dict[str, object] = {
        "tracks": len(results),
        "positives": len(positives),
        "negatives": len(negatives),
        "accept_rate": share([r.accepted for r in positives]),
        "false_accept": share([r.accepted for r in negatives]),
        "false_accept_wilson_upper": false_accept_bound(negatives, manifest),
        "n_eff_negatives": negative_effective_n(negatives),
        "n_min_negatives": stats.n_min(MAX_FALSE_ACCEPT),
        "line_metrics": line_metrics(positives),
        "line_metrics_all_references": line_metrics(positives, flavor="fitted"),
        "line_metrics_raw": line_metrics(positives, flavor="raw"),
        "reference_checks": [reference_payload(r) for r in positives if r.line_times],
        "by_split": {
            split: split_summary([r for r in results if r.split == split])
            for split in ("calib", "control")
        },
        "by_language": grouped(results, lambda r: r.language or "none"),
        "by_path": grouped(results, lambda r: r.path),
        "by_language_path": grouped(results, lambda r: f"{r.language or 'none'}/{r.path}"),
        "by_input_source": grouped(positives, lambda r: r.input_source),
        "by_kind": grouped(results, lambda r: r.kind),
        "reasons": reasons(results),
        "gpu_s_per_audio_minute": gpu_per_minute(results),
        "peak_vram_mib": dict(peak_vram),
        "wall_s_total": round(sum(r.wall_s for r in results), 1),
    }
    audit = next((r for r in results if r.id == manifest.audit_track), None)
    if audit is not None:
        summary["audit"] = {
            "status": audit.status,
            "confidence": audit.confidence,
            "lines_placed": (audit.lines_total or 0) - (audit.lines_unplaced or 0),
        }
    return summary


def split_summary(results: Sequence[TrackResult]) -> dict[str, object]:
    positives = [r for r in results if r.expected == "ok"]
    negatives = [r for r in results if r.expected != "ok"]
    return {
        "positives": len(positives),
        "negatives": len(negatives),
        "accept_rate": share([r.accepted for r in positives]),
        "false_accept": share([r.accepted for r in negatives]),
        "line_metrics": line_metrics(positives),
    }


def grouped(
    results: Sequence[TrackResult], group: Callable[[TrackResult], str]
) -> dict[str, object]:
    buckets: dict[str, list[TrackResult]] = defaultdict(list)
    for result in results:
        buckets[group(result)].append(result)
    return {name: split_summary(bucket) for name, bucket in sorted(buckets.items())}


def line_metrics(results: Sequence[TrackResult], *, flavor: Flavor = "usable") -> dict[str, object]:
    measured = [result for result in results if result.line_times]
    if flavor == "raw":
        deltas = [delta for result in measured for delta in result.raw_deltas_s]
    elif flavor == "fitted":
        deltas = [delta for result in measured for delta in result.deltas_s]
    else:
        deltas = [
            delta for result in measured if result.reference.usable for delta in result.deltas_s
        ]
    coverages = [result.coverage for result in results if result.coverage is not None]
    return {
        "lines": len(deltas),
        "tracks": len(measured),
        "reference_fitted": sum(1 for r in measured if r.reference.fit.applied),
        "reference_excluded": sum(1 for r in measured if not r.reference.usable),
        "median_abs_delta_s": round(stats.median([abs(d) for d in deltas]), 3) if deltas else None,
        **{f"acc@{t}": round(stats.accuracy_at(deltas, t), 4) for t in THRESHOLDS_S},
        "coverage": round(sum(coverages) / len(coverages), 4) if coverages else None,
    }


def reasons(results: Sequence[TrackResult]) -> dict[str, int]:
    counts: dict[str, int] = defaultdict(int)
    for result in results:
        counts[f"{result.status}/{result.reason}" if result.reason else result.status] += 1
    return dict(sorted(counts.items()))


def gpu_per_minute(results: Sequence[TrackResult]) -> dict[str, float]:
    unique_audio = [result for result in results if result.kind != "synthetic"]
    minutes = sum(result.audio_s for result in unique_audio) / 60.0
    if minutes <= 0:
        return {}
    totals: dict[str, float] = defaultdict(float)
    for result in unique_audio:
        for slot, seconds in result.gpu_s.items():
            totals[slot] += seconds
    return {slot: round(seconds / minutes, 3) for slot, seconds in sorted(totals.items())}


def false_accept_bound(negatives: Sequence[TrackResult], manifest: Manifest) -> float | None:
    if not negatives:
        return None
    accepted = sum(1 for result in negatives if result.accepted)
    return round(stats.wilson_upper(accepted, negative_effective_n(negatives)), 4)


def negative_effective_n(negatives: Sequence[TrackResult]) -> float:
    clusters: dict[str, int] = defaultdict(int)
    for result in negatives:
        clusters[result.cluster] += 1
    if not clusters:
        return 0.0
    return round(stats.n_eff(len(negatives), len(negatives) / len(clusters)), 1)


def share(flags: Sequence[bool]) -> float | None:
    return round(sum(flags) / len(flags), 4) if flags else None


def peak_vram_mib() -> dict[str, float]:
    try:
        import torch
    except ImportError:
        return {}
    if not torch.cuda.is_available():
        return {}
    return {"process": round(torch.cuda.max_memory_reserved() / 2**20, 1)}


def audio_duration_s(path: Path) -> float:
    try:
        return probe_duration(path)
    except CollectError as error:
        log.warning("no duration for %s: %s", path.name, error)
        return 0.0


def track_payload(result: TrackResult) -> dict[str, object]:
    return {
        "id": result.id,
        "kind": result.kind,
        "split": result.split,
        "cluster": result.cluster,
        "language": result.language,
        "input_source": result.input_source,
        "expected": result.expected,
        "status": result.status,
        "reason": result.reason,
        "confidence": result.confidence,
        "metrics": dict(result.metrics),
        "paths": dict(result.paths),
        "path": result.path,
        "acc@0.5": round(stats.accuracy_at(result.deltas_s, 0.5), 4) if result.deltas_s else None,
        "median_abs_delta_s": round(stats.median([abs(d) for d in result.deltas_s]), 3)
        if result.deltas_s
        else None,
        "acc@0.5_raw": round(stats.accuracy_at(result.raw_deltas_s, 0.5), 4)
        if result.line_times
        else None,
        "coverage": result.coverage,
        "reference": reference_payload(result) if result.line_times else None,
        "lines_total": result.lines_total,
        "lines_unplaced": result.lines_unplaced,
        "audio_s": round(result.audio_s, 1),
        "gpu_s": {slot: round(seconds, 2) for slot, seconds in result.gpu_s.items()},
        "wall_s": round(result.wall_s, 2),
        "line_times": {
            str(line): [round(produced, 3), round(expected, 3)]
            for line, (produced, expected) in result.line_times.items()
        },
        "reference_lines": [
            [round(line.start_s, 3), round(line.shortest_s, 3)] for line in result.reference_lines
        ],
        "line_deltas": {str(line): round(delta, 3) for line, delta in result.line_deltas.items()},
        "line_features": {
            str(line): [round(value, 4) for value in row]
            for line, row in result.line_features.items()
        },
    }


def reference_payload(result: TrackResult) -> dict[str, object]:
    check = result.reference
    return {
        "id": result.id,
        "scale": check.fit.scale,
        "shift_s": check.fit.shift_s,
        "inlier_share": check.fit.inlier_share,
        "applied": check.fit.applied,
        "flags": list(check.flags),
    }


def select(manifest: Manifest, ids: str, kinds: str, limit: int) -> list[Track]:
    wanted = {item for item in ids.split(",") if item}
    wanted_kinds = {item for item in kinds.split(",") if item}
    tracks = [
        track
        for track in manifest.tracks
        if (not wanted or track.id in wanted) and (not wanted_kinds or track.kind in wanted_kinds)
    ]
    tracks = tracks[:limit] if limit else tracks
    order = {track.cluster: index for index, track in enumerate(tracks)}
    return sorted(tracks, key=lambda track: (order[track.cluster], track.kind == "synthetic"))


def base_environment() -> dict[str, str]:
    return {
        **os.environ,
        "WORKER_NODE_NAME": os.environ.get("WORKER_NODE_NAME", "eval"),
        "NATS_URL": os.environ.get("NATS_URL", "nats://127.0.0.1:4222"),
        "NATS_USER": os.environ.get("NATS_USER", "eval"),
        "NATS_PASSWORD": os.environ.get("NATS_PASSWORD", "eval"),
    }


if __name__ == "__main__":
    sys.exit(main())
