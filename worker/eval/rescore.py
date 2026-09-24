from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from eval.harness import WORKER_ROOT, result_from_json, summarize, track_payload
from eval.manifest import load


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="eval.rescore")
    parser.add_argument("result", type=Path)
    parser.add_argument("--manifest", type=Path, default=WORKER_ROOT / "eval" / "manifest.json")
    args = parser.parse_args(argv)
    report = json.loads(args.result.read_text(encoding="utf-8"))
    results = [result_from_json(track) for track in report["tracks"]]
    report["summary"] = summarize(
        results, load(args.manifest), dict(report["summary"].get("peak_vram_mib", {}))
    )
    report["tracks"] = [track_payload(result) for result in results]
    args.result.write_text(
        json.dumps(report, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    print(json.dumps(report["summary"]["line_metrics"], ensure_ascii=False, indent=1))
    print(f"rescored {args.result}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
