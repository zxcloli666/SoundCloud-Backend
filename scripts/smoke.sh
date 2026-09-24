#!/usr/bin/env bash
set -uo pipefail

API_PORT="${API_PORT:-3399}"
JOBS_HEALTH_PORT="${JOBS_HEALTH_PORT:-3398}"
ADMIN_TOKEN="${ADMIN_TOKEN:-smoke-token}"
WAIT_SECONDS="${WAIT_SECONDS:-60}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

cd "$(git rev-parse --show-toplevel)"

SMOKE_DB="${SMOKE_DB:-soundcloud_smoke}"
SMOKE_PG_CONTAINER="${SMOKE_PG_CONTAINER:-scd-rewrite-pg}"
source "$(git rev-parse --show-toplevel)/scripts/local-env.sh"
BUILD_DATABASE_URL="${BUILD_DATABASE_URL:-postgres://soundcloud:devpw@127.0.0.1:55433/soundcloud_desktop}"
build() { ( DATABASE_URL="$BUILD_DATABASE_URL" OPS_DATABASE_URL="$BUILD_DATABASE_URL" "$@" ); }
export DATABASE_URL="postgres://soundcloud:devpw@127.0.0.1:55433/$SMOKE_DB"
export OPS_DATABASE_URL="$DATABASE_URL"
export SUBSCRIPTIONS_SNAPSHOT_DIR="${SUBSCRIPTIONS_SNAPSHOT_DIR:-$LOG_DIR/snapshots}"
mkdir -p "$SUBSCRIPTIONS_SNAPSHOT_DIR"

failures=0
pids=()

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }

cleanup() {
  for pid in "${pids[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null
  done
  wait 2>/dev/null
}
trap cleanup EXIT

port_is_free() {
  local port=$1 name=$2
  if curl -fsS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" \
    || curl -fsS -o /dev/null --max-time 1 "http://127.0.0.1:$port/live"; then
    fail "$name port $port already answers: a stale process would make this run lie"
    return 1
  fi
  return 0
}

alive() {
  local pid=$1 name=$2
  if kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  fail "$name exited during startup"
  return 1
}

wait_for() {
  local url=$1 name=$2 deadline=$((SECONDS + WAIT_SECONDS))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if curl -fsS -o /dev/null --max-time 2 "$url"; then
      pass "$name answered"
      return 0
    fi
    sleep 1
  done
  fail "$name never answered at $url"
  return 1
}

expect_status() {
  local url=$1 expected=$2 name=$3
  local got
  got=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$url")
  if [ "$got" = "$expected" ]; then
    pass "$name -> $got"
  else
    fail "$name -> $got, expected $expected"
  fi
}

expect_status_within() {
  local url=$1 name=$2 timeout=$3 budget=$4
  shift 4
  local answer got spent expected
  answer=$(curl -s -o /dev/null -w '%{http_code} %{time_total}' --max-time "$timeout" "$url")
  got=${answer%% *}
  spent=${answer##* }
  for expected in "$@"; do
    if [ "$got" = "$expected" ]; then
      if awk -v a="$spent" -v b="$budget" 'BEGIN { exit !(a < b) }'; then
        pass "$name -> $got in ${spent}s"
      else
        fail "$name -> $got but took ${spent}s, budget ${budget}s"
      fi
      return 0
    fi
  done
  fail "$name -> $got after ${spent}s, expected one of $*"
}

binary_of() {
  local crate=$1 name=$2 dir
  dir=$(cd "$crate" && cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  printf '%s/debug/%s\n' "$dir" "$name"
}

note "== fresh database =="
podman exec "$SMOKE_PG_CONTAINER" psql -U soundcloud -d postgres -q \
  -c "DROP DATABASE IF EXISTS $SMOKE_DB WITH (FORCE)" \
  -c "CREATE DATABASE $SMOKE_DB" > "$LOG_DIR/createdb.log" 2>&1 \
  || { fail "could not create $SMOKE_DB"; tail -5 "$LOG_DIR/createdb.log"; exit 1; }
pass "created $SMOKE_DB"

note "== building =="
(cd api && build cargo build --bin api > "$LOG_DIR/api-build.log" 2>&1) || { fail "api build"; tail -20 "$LOG_DIR/api-build.log"; exit 1; }
(cd jobs && build cargo build > "$LOG_DIR/jobs-build.log" 2>&1) || { fail "jobs build"; tail -20 "$LOG_DIR/jobs-build.log"; exit 1; }
(cd jobs && build cargo build --bin migrate > "$LOG_DIR/migrate-build.log" 2>&1) || { fail "migrate build"; tail -20 "$LOG_DIR/migrate-build.log"; exit 1; }
MIGRATE_BIN=$(binary_of jobs migrate)
[ -x "$MIGRATE_BIN" ] || { fail "migrate binary not found at $MIGRATE_BIN"; exit 1; }
if (cd jobs && "$MIGRATE_BIN" all > "$LOG_DIR/migrate.log" 2>&1); then
  pass "migrations applied to $SMOKE_DB"
else
  fail "migrations failed"
  tail -20 "$LOG_DIR/migrate.log"
  exit 1
fi

API_BIN=$(binary_of api api)
JOBS_BIN=$(binary_of jobs jobs)
[ -x "$API_BIN" ] || { fail "api binary not found at $API_BIN"; exit 1; }
[ -x "$JOBS_BIN" ] || { fail "jobs binary not found at $JOBS_BIN"; exit 1; }

note "== api alone, without the jobs service =="
port_is_free "$API_PORT" "api" || exit 1
(cd api && PORT="$API_PORT" ADMIN_TOKEN="$ADMIN_TOKEN" exec "$API_BIN" > "$LOG_DIR/api.log" 2>&1) &
API_PID="$!"
pids+=("$API_PID")
if ! wait_for "http://127.0.0.1:$API_PORT/health" "api health"; then
  alive "$API_PID" "api"
  note "--- api log ---"
  tail -20 "$LOG_DIR/api.log"
  exit 1
fi

expect_status "http://127.0.0.1:$API_PORT/health" 200 "GET /health"
expect_status "http://127.0.0.1:$API_PORT/tracks?q=never-seeded-needle" 401 "GET /tracks without a session"
expect_status "http://127.0.0.1:$API_PORT/admin/metrics" 401 "GET /admin/metrics without a token"
expect_status_within \
  "http://127.0.0.1:$API_PORT/resolve?url=https%3A%2F%2Fsoundcloud.com%2Fnobody%2Fnothing" \
  "GET /resolve with no SoundCloud channel configured" 30 5 502 504

code=$(curl -s -o "$LOG_DIR/metrics.txt" -w '%{http_code}' --max-time 10 \
  -H "x-admin-token: $ADMIN_TOKEN" "http://127.0.0.1:$API_PORT/admin/metrics")
if [ "$code" = "200" ] && grep -q "api_http_requests_total" "$LOG_DIR/metrics.txt"; then
  pass "GET /admin/metrics exports the request counter"
else
  fail "GET /admin/metrics -> $code without the expected metrics"
fi
for metric in api_pg_pool_connections api_pg_backends api_pg_sessions; do
  grep -q "$metric" "$LOG_DIR/metrics.txt" && pass "metric $metric" || fail "metric $metric missing"
done

note "== jobs alone, without the api process =="
port_is_free "$JOBS_HEALTH_PORT" "jobs" || exit 1
(cd jobs && JOBS_HEALTH_BIND="127.0.0.1:$JOBS_HEALTH_PORT" exec "$JOBS_BIN" > "$LOG_DIR/jobs.log" 2>&1) &
JOBS_PID="$!"
pids+=("$JOBS_PID")
if wait_for "http://127.0.0.1:$JOBS_HEALTH_PORT/live" "jobs liveness"; then
  wait_for "http://127.0.0.1:$JOBS_HEALTH_PORT/ready" "jobs readiness"
  code=$(curl -s -o "$LOG_DIR/jobs-metrics.txt" -w '%{http_code}' --max-time 10 \
    "http://127.0.0.1:$JOBS_HEALTH_PORT/metrics")
  if [ "$code" = "200" ] && grep -q "jobs_queue_depth" "$LOG_DIR/jobs-metrics.txt"; then
    pass "GET /metrics exports the queue depth"
  else
    fail "jobs /metrics -> $code without the expected metrics"
  fi
else
  alive "$JOBS_PID" "jobs"
  note "--- jobs log ---"
  tail -20 "$LOG_DIR/jobs.log"
fi

degraded_api() {
  local name=$1 expected=$2 port=$((API_PORT + 10))
  shift 2
  port_is_free "$port" "api ($name)" || return 1
  (cd api && PORT="$port" ADMIN_TOKEN="$ADMIN_TOKEN" exec env "$@" "$API_BIN" \
    > "$LOG_DIR/api-$name.log" 2>&1) &
  local pid="$!"
  pids+=("$pid")
  local deadline=$((SECONDS + WAIT_SECONDS)) answered=no
  while [ "$SECONDS" -lt "$deadline" ]; do
    if curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$port/health"; then
      answered=yes
      break
    fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
  done
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  if [ "$answered" = "$expected" ]; then
    pass "api without $name: serves=$answered, as recorded"
  else
    fail "api without $name: serves=$answered, recorded $expected"
    tail -5 "$LOG_DIR/api-$name.log"
  fi
}

degraded_jobs() {
  local name=$1 expected=$2 port=$((JOBS_HEALTH_PORT + 10))
  shift 2
  port_is_free "$port" "jobs ($name)" || return 1
  (cd jobs && JOBS_HEALTH_BIND="127.0.0.1:$port" exec env "$@" "$JOBS_BIN" \
    > "$LOG_DIR/jobs-$name.log" 2>&1) &
  local pid="$!"
  pids+=("$pid")
  local deadline=$((SECONDS + WAIT_SECONDS)) usable=no
  while [ "$SECONDS" -lt "$deadline" ]; do
    if curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$port/ready"; then
      usable=yes
      break
    fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
  done
  if [ "$usable" = yes ] && ! kill -0 "$pid" 2>/dev/null; then
    usable=no
  fi
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  if [ "$usable" = "$expected" ]; then
    pass "jobs without $name: takes work=$usable, as recorded"
  else
    fail "jobs without $name: takes work=$usable, recorded $expected"
    tail -3 "$LOG_DIR/jobs-$name.log"
  fi
}

degraded_sc() {
  local name=$1 port=$((API_PORT + 20))
  shift
  port_is_free "$port" "api ($name)" || return 1
  (cd api && PORT="$port" ADMIN_TOKEN="$ADMIN_TOKEN" exec env "$@" "$API_BIN" \
    > "$LOG_DIR/api-$name.log" 2>&1) &
  local pid="$!"
  pids+=("$pid")
  local deadline=$((SECONDS + WAIT_SECONDS)) answered=no
  while [ "$SECONDS" -lt "$deadline" ]; do
    if curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$port/health"; then
      answered=yes
      break
    fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
  done
  if [ "$answered" != yes ]; then
    fail "api with $name: never came up"
    tail -5 "$LOG_DIR/api-$name.log"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    return 1
  fi
  pass "api with $name: local routes answer"
  expect_status "http://127.0.0.1:$port/tracks?q=never-seeded-needle" 401 \
    "GET /tracks with $name"
  expect_status_within \
    "http://127.0.0.1:$port/resolve?url=https%3A%2F%2Fsoundcloud.com%2Fnobody%2Fnothing" \
    "GET /resolve with $name" 30 5 502 504
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

note "== soundcloud channels taken away: local routes must not care =="
degraded_sc relay-down CALL_CONTROL_ENDPOINT="http://127.0.0.1:1" CALL_RELAY_SECRET="smoke"
degraded_sc proxy-down SC_PROXY_URL="http://127.0.0.1:1"
degraded_sc both-sc-down CALL_CONTROL_ENDPOINT="http://127.0.0.1:1" \
  CALL_RELAY_SECRET="smoke" SC_PROXY_URL="http://127.0.0.1:1"

note "== degradations: each dependency taken away in turn =="
degraded_api redis yes REDIS_URL="redis://127.0.0.1:1"
degraded_api nats yes NATS_URL="nats://127.0.0.1:1"
degraded_api qdrant no QDRANT_URL="http://127.0.0.1:1"
degraded_jobs redis yes REDIS_URL="redis://127.0.0.1:1"
degraded_jobs nats no NATS_URL="nats://127.0.0.1:1"
degraded_jobs qdrant no QDRANT_URL="http://127.0.0.1:1"

note "== logs in $LOG_DIR =="
if [ "$failures" -eq 0 ]; then
  note "smoke: ok"
  exit 0
fi
note "smoke: $failures failed"
exit 1
