#!/usr/bin/env bash
set -uo pipefail

cd "$(git rev-parse --show-toplevel)"

CORE_DIR="${CORE_DIR:-api/migrations}"
OPS_DIR="${OPS_DIR:-api/migrations-ops}"
REQUIRED_FROM="${REQUIRED_FROM:-api/src/db/mod.rs}"

failures=0
checked_databases=0
checked_migrations=0
SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }
skip() { printf 'skip %s\n' "$*"; }

required_version=$(grep -oE 'REQUIRED_CORE_SCHEMA_VERSION: i64 = [0-9]+' "$REQUIRED_FROM" \
  | grep -oE '[0-9]+$')
if [ -z "$required_version" ]; then
  fail "could not read REQUIRED_CORE_SCHEMA_VERSION from $REQUIRED_FROM"
  exit 2
fi
note "api refuses to serve below core migration $required_version"

file_checksums() {
  local dir=$1 path base version
  for path in "$dir"/*.sql; do
    [ -e "$path" ] || continue
    base=$(basename "$path" .sql)
    version=${base%%_*}
    printf '%s %s\n' "$((10#$version))" "$(sha384sum "$path" | cut -d' ' -f1)"
  done
}

applied_checksums() {
  "$@" -tAqX -c \
    "SELECT version, encode(checksum, 'hex') FROM _sqlx_migrations ORDER BY version" \
    < /dev/null 2>/dev/null | tr '|' ' '
}

inspect() {
  local label=$1 raw=$2 kind=$3
  local -a command
  read -r -a command <<< "$raw"

  if ! "${command[@]}" -tAqX -c 'SELECT 1' > /dev/null 2>&1 < /dev/null; then
    fail "$label: cannot run a query at all — check the command"
    return 1
  fi
  checked_databases=$((checked_databases + 1))

  local applied
  applied=$(applied_checksums "${command[@]}")
  if [ -z "$applied" ]; then
    fail "$label: _sqlx_migrations is empty — this database was never migrated by jobs-migrate"
    return 1
  fi

  local -A on_disk=()
  local version checksum
  while read -r version checksum; do
    [ -n "$version" ] && on_disk["$version"]="$checksum"
  done < <(file_checksums "$CORE_DIR"; file_checksums "$OPS_DIR")

  local mismatched=0 unknown=0 counted=0 max_core=-1
  while read -r version checksum; do
    [ -n "$version" ] || continue
    counted=$((counted + 1))
    [ "$version" -lt 9000 ] && [ "$version" -gt "$max_core" ] && max_core="$version"
    local expected="${on_disk[$version]:-}"
    if [ -z "$expected" ]; then
      unknown=$((unknown + 1))
      fail "$label: migration $version is applied but no file carries that number"
    elif [ "$expected" != "$checksum" ]; then
      mismatched=$((mismatched + 1))
      fail "$label: migration $version was edited after it was applied"
    fi
  done <<< "$applied"
  checked_migrations=$((checked_migrations + counted))

  if [ "$counted" -eq 0 ]; then
    fail "$label: nothing to compare — the check proved nothing"
    return 1
  fi
  if [ "$mismatched" -eq 0 ] && [ "$unknown" -eq 0 ]; then
    pass "$label: $counted applied migrations all match their files"
  fi

  local -a missing=()
  for version in "${!on_disk[@]}"; do
    if [ "$kind" = core ] && [ "$version" -ge 9000 ]; then continue; fi
    if [ "$kind" = ops ] && [ "$version" -lt 9000 ]; then continue; fi
    [ "$version" -le "$max_core" ] || [ "$version" -ge 9000 ] || continue
    grep -qE "^$version " <<< "$applied" || missing+=("$version")
  done
  if [ "$kind" = core ]; then
    if [ "${#missing[@]}" -eq 0 ]; then
      pass "$label: no gap below the applied level $max_core"
    else
      fail "$label: applied up to $max_core but missing ${missing[*]}"
    fi
    if [ "$max_core" -ge "$required_version" ]; then
      pass "$label: core level $max_core satisfies the $required_version api needs"
    else
      fail "$label: core level $max_core is below the $required_version api needs"
    fi
    printf '%s\n' "$max_core" > "$SCRATCH/level-$label"
  fi
  return 0
}

note ""
note "== core databases =="
core_labels=()
if [ -n "${PSQL_MAIN:-}" ]; then
  inspect main "$PSQL_MAIN" core && core_labels+=(main)
else
  fail "PSQL_MAIN is not set: there is nothing to check"
fi
if [ -n "${PSQL_STAR:-}" ]; then
  inspect star "$PSQL_STAR" core && core_labels+=(star)
else
  skip "PSQL_STAR not set: the second serving node was not checked"
fi

if [ "${#core_labels[@]}" -ge 2 ]; then
  levels=$(for label in "${core_labels[@]}"; do cat "$SCRATCH/level-$label"; done | sort -u | tr '\n' ' ')
  if [ "$(wc -w <<< "$levels")" -eq 1 ]; then
    pass "both serving nodes stand at core level $levels"
  else
    fail "serving nodes stand at different core levels: $levels"
  fi
fi

note ""
note "== ops database =="
if [ -n "${PSQL_OPS:-}" ]; then
  inspect ops "$PSQL_OPS" ops
else
  skip "PSQL_OPS not set: ops is optional, but nothing was checked"
fi

note ""
note "== durables drained =="
if [ -n "${NATS_MONITOR:-}" ]; then
  if curl -fsS --max-time 10 "$NATS_MONITOR/jsz?consumers=1&streams=1" -o "$SCRATCH/jsz.json"; then
    busy=$(python3 - "$SCRATCH/jsz.json" <<'PY'
import json, sys
with open(sys.argv[1]) as handle:
    data = json.load(handle)
seen = 0
busy = []
for account in data.get("account_details", []) or [data]:
    for stream in account.get("stream_detail", []) or []:
        for consumer in stream.get("consumer_detail", []) or []:
            seen += 1
            pending = consumer.get("num_pending", 0) + consumer.get("num_ack_pending", 0)
            if pending:
                busy.append(f"{stream.get('name')}/{consumer.get('name')}={pending}")
print(seen, " ".join(busy))
PY
)
    seen=${busy%% *}
    rest=${busy#* }
    if [ "${seen:-0}" -eq 0 ]; then
      fail "the broker reported no consumers at all: the drain check proved nothing"
    elif [ -z "${rest// /}" ]; then
      pass "all $seen durables are drained"
    else
      fail "durables still hold work: $rest"
    fi
  else
    fail "could not read $NATS_MONITOR/jsz"
  fi
else
  skip "NATS_MONITOR not set: durable drain was not checked"
fi

note ""
note "проверено $checked_migrations миграций на $checked_databases базах"
if [ "$checked_databases" -eq 0 ] || [ "$checked_migrations" -eq 0 ]; then
  note "preflight: ничего не проверено"
  exit 2
fi
if [ "$failures" -eq 0 ]; then
  note "preflight: ok"
  exit 0
fi
note "preflight: $failures failed"
exit 1
