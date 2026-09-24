#!/usr/bin/env bash
set -uo pipefail

cd "$(git rev-parse --show-toplevel)"

ALL_SUITES="smoke integration load chaos soak"
SUITES="${*:-$ALL_SUITES}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

note() { printf '%s\n' "$*"; }
started=$SECONDS
declare -A verdict
declare -A spent

for suite in $SUITES; do
  script="scripts/$suite.sh"
  if [ ! -x "$script" ]; then
    note "no such suite: $suite"
    verdict[$suite]="missing"
    continue
  fi
  note ""
  note "========== $suite =========="
  at=$SECONDS
  if LOG_DIR="$LOG_DIR/$suite" bash -c "mkdir -p '$LOG_DIR/$suite'; exec '$script'" 2>&1 | tee "$LOG_DIR/$suite.out"; then
    verdict[$suite]="ok"
  else
    verdict[$suite]="failed"
  fi
  spent[$suite]=$((SECONDS - at))
done

note ""
note "========== acceptance =========="
broken=0
for suite in $SUITES; do
  state="${verdict[$suite]:-skipped}"
  printf '%-12s %-8s %ss\n' "$suite" "$state" "${spent[$suite]:-0}"
  [ "$state" = "ok" ] || broken=$((broken + 1))
done
note "total $((SECONDS - started))s, logs in $LOG_DIR"

[ "$broken" -eq 0 ] && { note "acceptance: ok"; exit 0; }
note "acceptance: $broken suite(s) failed"
exit 1
