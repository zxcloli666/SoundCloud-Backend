#!/usr/bin/env bash
set -uo pipefail

API_PORT="${API_PORT:-3397}"
LOAD_DB="${LOAD_DB:-soundcloud_load}"
LOAD_PG_CONTAINER="${LOAD_PG_CONTAINER:-scd-rewrite-pg}"
LOAD_REDIS_CONTAINER="${LOAD_REDIS_CONTAINER:-scd-auth-redis}"
LOAD_REDIS_DB="${LOAD_REDIS_DB:-9}"
COLD_NEEDLES="${COLD_NEEDLES:-200}"
POOL_WAIT_BUDGET_MS="${POOL_WAIT_BUDGET_MS:-50}"
CONCURRENCY="${CONCURRENCY:-16}"
AUTH_BURST_CONCURRENCY="${AUTH_BURST_CONCURRENCY:-64}"
REQUESTS="${REQUESTS:-400}"
P99_BUDGET_MS="${P99_BUDGET_MS:-500}"
ERROR_BUDGET="${ERROR_BUDGET:-0}"
PROFILE="${PROFILE:-release}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

cd "$(git rev-parse --show-toplevel)"
source "$(git rev-parse --show-toplevel)/scripts/local-env.sh"
BUILD_DATABASE_URL="${BUILD_DATABASE_URL:-postgres://soundcloud:devpw@127.0.0.1:55433/soundcloud_desktop}"
build() { ( DATABASE_URL="$BUILD_DATABASE_URL" OPS_DATABASE_URL="$BUILD_DATABASE_URL" "$@" ); }
export DATABASE_URL="postgres://soundcloud:devpw@127.0.0.1:55433/$LOAD_DB"
export OPS_DATABASE_URL="$DATABASE_URL"
export REDIS_URL="${LOAD_REDIS_URL:-redis://127.0.0.1:6379/$LOAD_REDIS_DB}"
export ADMIN_TOKEN="${ADMIN_TOKEN:-load-admin}"
export SUBSCRIPTIONS_SNAPSHOT_DIR="$LOG_DIR/snapshots"
mkdir -p "$SUBSCRIPTIONS_SNAPSHOT_DIR"

failures=0
api_pid=""

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }

cleanup() { [ -n "$api_pid" ] && kill "$api_pid" 2>/dev/null; wait 2>/dev/null; }
trap cleanup EXIT

psql_load() { podman exec -i "$LOAD_PG_CONTAINER" psql -U soundcloud -d "$LOAD_DB" -qtAX "$@"; }
redis_load() { podman exec -i "$LOAD_REDIS_CONTAINER" redis-cli -n "$LOAD_REDIS_DB" "$@"; }

binary_of() {
  local crate=$1 name=$2 dir
  dir=$(cd "$crate" && cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  printf '%s/%s/%s\n' "$dir" "$PROFILE" "$name"
}

build_flags() {
  [ "$PROFILE" = "release" ] && printf -- '--release' || printf ''
}

note "== fresh database with selective data =="
podman exec "$LOAD_PG_CONTAINER" psql -U soundcloud -d postgres -q \
  -c "DROP DATABASE IF EXISTS $LOAD_DB WITH (FORCE)" \
  -c "CREATE DATABASE $LOAD_DB" > "$LOG_DIR/createdb.log" 2>&1 \
  || { fail "could not create $LOAD_DB"; tail -5 "$LOG_DIR/createdb.log"; exit 1; }
pass "created $LOAD_DB"

note "== schema and session =="
(cd jobs && build cargo build $(build_flags) --bin migrate > "$LOG_DIR/migrate-build.log" 2>&1) || { fail "migrate build"; exit 1; }
MIGRATE_BIN=$(binary_of jobs migrate)
(cd jobs && "$MIGRATE_BIN" all > "$LOG_DIR/migrate.log" 2>&1) || { fail "migrations"; tail -10 "$LOG_DIR/migrate.log"; exit 1; }
pass "$LOAD_DB is at the current schema"

SESSION_ID=$(psql_load -c "
  WITH connection AS (
      INSERT INTO soundcloud_connections (
          id, soundcloud_user_id, access_token, refresh_token, expires_at, scope
      ) VALUES (
          gen_random_uuid(), '17', 'load', 'load', now() + interval '1 day', ''
      )
      ON CONFLICT DO NOTHING
      RETURNING id
  ), picked AS (
      SELECT id FROM connection
      UNION ALL
      SELECT id FROM soundcloud_connections WHERE soundcloud_user_id = '17' LIMIT 1
  )
  INSERT INTO sessions (id, soundcloud_connection_id)
  SELECT gen_random_uuid(), id FROM picked LIMIT 1
  RETURNING id")
[ -n "$SESSION_ID" ] || { fail "could not create a load session"; exit 1; }
pass "session $SESSION_ID"

note "== seeding =="
podman exec -i "$LOAD_PG_CONTAINER" psql -U soundcloud -d "$LOAD_DB" -q -v ON_ERROR_STOP=1 \
  < scripts/seed-plan-fixtures.sql > "$LOG_DIR/seed.log" 2>&1 \
  || { fail "seeding failed"; tail -5 "$LOG_DIR/seed.log"; exit 1; }
TOTAL=$(psql_load -c "SELECT count(*) FROM tracks WHERE sharing = 'public'")
pass "$TOTAL public tracks"

TRACK_URN=$(psql_load -c "SELECT sc_track_id FROM tracks WHERE sharing = 'public' AND deleted_at IS NULL LIMIT 1")
NEEDLE=$(psql_load -c "SELECT word FROM (
    SELECT split_part(title_normalized, ' ', 1) AS word, count(*) AS hits
    FROM tracks WHERE sharing = 'public' GROUP BY 1
  ) AS words WHERE hits * 100 < $TOTAL * 25 ORDER BY hits DESC LIMIT 1")
if [ -z "$NEEDLE" ]; then
  fail "no needle matches under a quarter of the catalog: the fixture is degenerate and any search number would be meaningless"
  exit 1
fi
MATCHES=$(psql_load -c "SELECT count(*) FROM tracks WHERE sharing = 'public' AND title_normalized LIKE '%$NEEDLE%'")
pass "needle '$NEEDLE' matches $MATCHES of $TOTAL"
USER_NEEDLE=$(psql_load -c "SELECT split_part(username_normalized, ' ', 1) FROM users LIMIT 1")

COLD_NEEDLE_FILE="$LOG_DIR/cold-needles.txt"
psql_load -c "SELECT normalized_name FROM artists
  WHERE merged_into IS NULL
    AND (track_count_primary > 0 OR track_count_featured > 0)
  ORDER BY id LIMIT $COLD_NEEDLES" > "$COLD_NEEDLE_FILE"
COLD_AVAILABLE=$(sort -u "$COLD_NEEDLE_FILE" | grep -c . || true)
if [ "$COLD_AVAILABLE" -lt 50 ]; then
  fail "only $COLD_AVAILABLE distinct artist needles: a cold-cache number off so few queries would be noise"
  exit 1
fi
COLD_HITS=$(psql_load -c "SELECT count(*) FROM artists
  WHERE merged_into IS NULL
    AND (track_count_primary > 0 OR track_count_featured > 0)
    AND normalized_name = (SELECT normalized_name FROM artists
      WHERE merged_into IS NULL AND (track_count_primary > 0 OR track_count_featured > 0)
      ORDER BY id LIMIT 1)")
if [ "${COLD_HITS:-0}" -lt 1 ]; then
  fail "the cold-cache needles match nothing: the number would measure an empty search"
  exit 1
fi
pass "$COLD_AVAILABLE distinct artist needles for the cold pass, each matching at least $COLD_HITS"

if redis_load FLUSHDB > "$LOG_DIR/redis-flush.log" 2>&1; then
  pass "redis db $LOAD_REDIS_DB flushed, the run starts cold"
else
  fail "could not flush redis db $LOAD_REDIS_DB"
  exit 1
fi

note "== api =="
(cd api && build cargo build $(build_flags) --bin api > "$LOG_DIR/api-build.log" 2>&1) || { fail "api build"; exit 1; }
API_BIN=$(binary_of api api)
if curl -fsS -o /dev/null --max-time 1 "http://127.0.0.1:$API_PORT/health"; then
  fail "port $API_PORT already answers"
  exit 1
fi
(cd api && PORT="$API_PORT" exec "$API_BIN" > "$LOG_DIR/api.log" 2>&1) &
api_pid="$!"
deadline=$((SECONDS + 60))
until curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$API_PORT/health"; do
  kill -0 "$api_pid" 2>/dev/null || { fail "api exited"; tail -10 "$LOG_DIR/api.log"; exit 1; }
  [ "$SECONDS" -lt "$deadline" ] || { fail "api never answered"; exit 1; }
  sleep 1
done
pass "api is up"

drive() {
  local name="$1" path="$2" concurrency="${3:-$CONCURRENCY}"
  local samples="$LOG_DIR/$name.times"
  : > "$samples"
  seq "$REQUESTS" | xargs -P "$concurrency" -I{} \
    curl -s -o /dev/null -w '%{http_code} %{time_total}\n' --max-time 30 \
      -H "x-session-id: $SESSION_ID" "http://127.0.0.1:$API_PORT$path" >> "$samples"
  analyze "$samples" "$name"
}

drive_mixed() {
  local name="$1" concurrency="${2:-$CONCURRENCY}"
  local samples="$LOG_DIR/$name.times" plan="$LOG_DIR/$name.paths"
  : > "$samples"
  : > "$plan"
  local request
  for request in $(seq "$REQUESTS"); do
    case $((request % 5)) in
      0) printf '%s\n' "/health" >> "$plan" ;;
      1) printf '%s\n' "/tracks?q=$NEEDLE&limit=30" >> "$plan" ;;
      2) printf '%s\n' "/tracks/$TRACK_URN" >> "$plan" ;;
      3) printf '%s\n' "/discover/tags" >> "$plan" ;;
      *) printf '%s\n' "/me" >> "$plan" ;;
    esac
  done
  xargs -P "$concurrency" -I{} -a "$plan" \
    curl -s -o /dev/null -w '%{http_code} %{time_total}\n' --max-time 30 \
      -H "x-session-id: $SESSION_ID" "http://127.0.0.1:$API_PORT{}" >> "$samples"
  analyze "$samples" "$name"
}

drive_plan() {
  local name="$1" plan="$2" concurrency="$3"
  local samples="$LOG_DIR/$name.times"
  : > "$samples"
  xargs -P "$concurrency" -I{} -a "$plan" \
    curl -s -o /dev/null -w '%{http_code} %{time_total}\n' --max-time 30 \
      -H "x-session-id: $SESSION_ID" "http://127.0.0.1:$API_PORT{}" >> "$samples"
  analyze "$samples" "$name"
}

drive_cold_then_warm() {
  local concurrency="${1:-$CONCURRENCY}"
  local plan="$LOG_DIR/cold.paths"
  sort -u "$COLD_NEEDLE_FILE" \
    | awk '{ gsub(/ /, "%20"); printf "/search/db/artists?q=%s&limit=30\n", $0 }' > "$plan"
  redis_load FLUSHDB > /dev/null 2>&1
  drive_plan cold_artist_search "$plan" "$concurrency"
  drive_plan warm_artist_search "$plan" "$concurrency"
  cache_actually_helps "$LOG_DIR/cold_artist_search.times" "$LOG_DIR/warm_artist_search.times"
}

cache_actually_helps() {
  python3 - "$1" "$2" <<'PY'
import sys
def median(path):
    times = []
    for line in open(path):
        parts = line.split()
        if len(parts) == 2 and parts[0].startswith("2"):
            times.append(float(parts[1]) * 1000.0)
    times.sort()
    return times[len(times) // 2] if times else float("nan")
MARGIN = 0.5
cold, warm = median(sys.argv[1]), median(sys.argv[2])
verdict = "ok  " if warm <= cold * MARGIN else "FAIL"
print(
    f"{verdict} cache pays for itself          cold p50={cold:.1f}ms warm p50={warm:.1f}ms "
    f"(warm must be at most {MARGIN:.0%} of cold)"
)
sys.exit(0 if verdict == "ok  " else 1)
PY
  [ $? -eq 0 ] || failures=$((failures + 1))
}

pool_wait_within_budget() {
  local metrics="$LOG_DIR/pool-wait.txt" code
  code=$(curl -s -o "$metrics" -w '%{http_code}' --max-time 10 \
    -H "x-admin-token: $ADMIN_TOKEN" "http://127.0.0.1:$API_PORT/admin/metrics")
  if [ "$code" != "200" ]; then
    fail "pool wait: /admin/metrics answered $code"
    return
  fi
  local last bad
  last=$(awk '/^api_pg_pool_wait_last_seconds /{ print $2 }' "$metrics" | tail -1)
  bad=$(awk '/^api_pg_pool_wait_seconds_count\{outcome="(error|timeout)"\}/{ sum += $2 } END { print sum + 0 }' "$metrics")
  if [ -z "$last" ]; then
    fail "pool wait: the probe reported nothing"
    return
  fi
  if awk -v a="$last" -v b="$POOL_WAIT_BUDGET_MS" 'BEGIN { exit !(a * 1000 < b) }' && [ "$bad" = "0" ]; then
    pass "$(printf 'pool wait                     last=%.3fms refused=%s' "$(awk -v a="$last" 'BEGIN { print a * 1000 }')" "$bad")"
  else
    fail "$(printf 'pool wait                     last=%.3fms refused=%s budget=%sms' "$(awk -v a="$last" 'BEGIN { print a * 1000 }')" "$bad" "$POOL_WAIT_BUDGET_MS")"
  fi
}

analyze() {
  local samples="$1" name="$2"
  python3 - "$samples" "$name" "$P99_BUDGET_MS" "$ERROR_BUDGET" <<'PY'
import sys
path, name, budget_ms, error_budget = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])
times, errors = [], 0
for line in open(path):
    parts = line.split()
    if len(parts) != 2:
        continue
    code, seconds = parts
    if not code.startswith("2"):
        errors += 1
    times.append(float(seconds) * 1000.0)
times.sort()
def pick(q):
    return times[min(len(times) - 1, int(len(times) * q))] if times else float("nan")
rate = errors / len(times) * 100 if times else 100.0
verdict = "ok  " if pick(0.99) <= budget_ms and rate <= error_budget else "FAIL"
print(f"{verdict} {name:28} n={len(times):4} p50={pick(0.5):7.1f}ms p95={pick(0.95):7.1f}ms p99={pick(0.99):7.1f}ms errors={rate:.1f}%")
sys.exit(0 if verdict == "ok  " else 1)
PY
  [ $? -eq 0 ] || failures=$((failures + 1))
}

note "== load: ${REQUESTS} requests, concurrency ${CONCURRENCY}, ${PROFILE} build, p99 budget ${P99_BUDGET_MS}ms =="
drive health "/health"
drive track_search "/tracks?q=$NEEDLE&limit=30"
drive track_detail "/tracks/$TRACK_URN"
drive discover_tags "/discover/tags"
drive user_search "/users?q=$USER_NEEDLE&limit=30"

note "== auth burst: the session path at ${AUTH_BURST_CONCURRENCY}x =="
drive auth_burst "/me" "$AUTH_BURST_CONCURRENCY"

note "== mixed workload: five routes interleaved at concurrency ${CONCURRENCY} =="
drive_mixed mixed

note "== cold cache: ${COLD_AVAILABLE} distinct searches, then the same ones warm =="
drive_cold_then_warm

note "== waiting for a pooled connection =="
pool_wait_within_budget

note "== logs in $LOG_DIR =="
[ "$failures" -eq 0 ] && { note "load: ok"; exit 0; }
note "load: $failures over budget"
exit 1
