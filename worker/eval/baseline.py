from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

WORKER_ROOT = Path(__file__).resolve().parent.parent
BASELINE = WORKER_ROOT / "eval" / "baseline.json"
KEPT_KEYS = ("sync_version", "strategy", "device", "manifest_version", "audit_track", "summary")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="eval.baseline")
    parser.add_argument("result", type=Path)
    parser.add_argument("--out", type=Path, default=BASELINE)
    args = parser.parse_args(argv)
    report = json.loads(args.result.read_text(encoding="utf-8"))
    if report.get("strategy") != "anchored":
        parser.error("the baseline is the shipped strategy: pass an .anchored.json result")
    baseline = {key: report[key] for key in KEPT_KEYS}
    baseline["source"] = args.result.name
    args.out.write_text(json.dumps(baseline, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    summary = baseline["summary"]
    print(
        json.dumps(
            {
                "sync_version": baseline["sync_version"],
                "accept_rate": summary["accept_rate"],
                "false_accept_wilson_upper": summary["false_accept_wilson_upper"],
                "control_acc@0.5": summary["by_split"]["control"]["line_metrics"]["acc@0.5"],
                "gpu_s_per_audio_minute": summary["gpu_s_per_audio_minute"],
                "audit": summary.get("audit"),
            },
            ensure_ascii=False,
            indent=1,
        )
    )
    print(f"written {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
