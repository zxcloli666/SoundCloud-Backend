#!/usr/bin/env bash
set -uo pipefail

API_PORT="${API_PORT:-3390}"
JOBS_HEALTH_PORT="${JOBS_HEALTH_PORT:-3391}"
STREAMING_PORT="${STREAMING_PORT:-3392}"
ADMIN_TOKEN="${ADMIN_TOKEN:-integration-token}"
WAIT_SECONDS="${WAIT_SECONDS:-90}"
INGRESS_SECONDS="${INGRESS_SECONDS:-25}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

cd "$(git rev-parse --show-toplevel)"

INTEGRATION_DB="${INTEGRATION_DB:-soundcloud_integration}"
INTEGRATION_PG_CONTAINER="${INTEGRATION_PG_CONTAINER:-scd-rewrite-pg}"
INTEGRATION_NATS_CONTAINER="${INTEGRATION_NATS_CONTAINER:-scd-integration-nats}"
INTEGRATION_NATS_PORT="${INTEGRATION_NATS_PORT:-4225}"
INTEGRATION_NATS_MONITOR_PORT="${INTEGRATION_NATS_MONITOR_PORT:-8225}"
INTEGRATION_REDIS_DB="${INTEGRATION_REDIS_DB:-10}"

source "$(git rev-parse --show-toplevel)/scripts/local-env.sh"
BUILD_DATABASE_URL="${BUILD_DATABASE_URL:-postgres://soundcloud:devpw@127.0.0.1:55433/soundcloud_desktop}"
build() { ( DATABASE_URL="$BUILD_DATABASE_URL" OPS_DATABASE_URL="$BUILD_DATABASE_URL" "$@" ); }
export DATABASE_URL="postgres://soundcloud:devpw@127.0.0.1:55433/$INTEGRATION_DB"
export OPS_DATABASE_URL="$DATABASE_URL"
export NATS_URL="nats://127.0.0.1:$INTEGRATION_NATS_PORT"
export REDIS_URL="redis://127.0.0.1:6379/$INTEGRATION_REDIS_DB"
export STREAMING_SERVICE_URL="http://127.0.0.1:$STREAMING_PORT"
export SUBSCRIPTIONS_SNAPSHOT_DIR="$LOG_DIR/snapshots"
mkdir -p "$SUBSCRIPTIONS_SNAPSHOT_DIR"

SESSION_ID="11111111-1111-4111-8111-111111111111"
CONNECTION_ID="22222222-2222-4222-8222-222222222222"
SEEDED_TRACK="700000001"
CLAIMED_TRACK="700000002"
UNCLAIMED_TRACK="700000003"

failures=0
api_pid=""
jobs_pid=""
streaming_pid=""

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }

cleanup() {
  for pid in "$api_pid" "$jobs_pid" "$streaming_pid"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null
  done
  wait 2>/dev/null
  podman rm -f "$INTEGRATION_NATS_CONTAINER" > /dev/null 2>&1
}
trap cleanup EXIT

psql_db() { podman exec -i "$INTEGRATION_PG_CONTAINER" psql -U soundcloud -d "$INTEGRATION_DB" -qtAX "$@" < /dev/null; }

binary_of() {
  local crate=$1 name=$2 dir
  dir=$(cd "$crate" && cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  printf '%s/debug/%s\n' "$dir" "$name"
}

port_is_free() {
  local port=$1 name=$2
  if curl -fsS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" \
    || curl -fsS -o /dev/null --max-time 1 "http://127.0.0.1:$port/live"; then
    fail "$name port $port already answers: a stale process would make this run lie"
    return 1
  fi
  return 0
}

wait_for() {
  local url=$1 name=$2 pid=$3 deadline=$((SECONDS + WAIT_SECONDS))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if curl -fs -o /dev/null --max-time 2 "$url" 2>/dev/null; then
      pass "$name answered"
      return 0
    fi
    kill -0 "$pid" 2>/dev/null || { fail "$name exited during startup"; return 1; }
    sleep 1
  done
  fail "$name never answered at $url"
  return 1
}

status_of() { curl -s -o /dev/null -w '%{http_code}' --max-time 15 "$@"; }

note "== a broker of our own, so no leftover message can answer for us =="
podman rm -f "$INTEGRATION_NATS_CONTAINER" > /dev/null 2>&1
podman run -d --name "$INTEGRATION_NATS_CONTAINER" \
  -p "127.0.0.1:$INTEGRATION_NATS_PORT:4222" \
  -p "127.0.0.1:$INTEGRATION_NATS_MONITOR_PORT:8222" \
  docker.io/library/nats:2-alpine -js -m 8222 > "$LOG_DIR/nats.log" 2>&1 \
  || { fail "could not start $INTEGRATION_NATS_CONTAINER"; tail -5 "$LOG_DIR/nats.log"; exit 1; }
deadline=$((SECONDS + 30))
until (exec 3<>"/dev/tcp/127.0.0.1/$INTEGRATION_NATS_PORT") 2>/dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || { fail "$INTEGRATION_NATS_CONTAINER never listened"; exit 1; }
  sleep 1
done
pass "$INTEGRATION_NATS_CONTAINER on $INTEGRATION_NATS_PORT"

note "== fresh database =="
podman exec "$INTEGRATION_PG_CONTAINER" psql -U soundcloud -d postgres -q \
  -c "DROP DATABASE IF EXISTS $INTEGRATION_DB WITH (FORCE)" \
  -c "CREATE DATABASE $INTEGRATION_DB" > "$LOG_DIR/createdb.log" 2>&1 \
  || { fail "could not create $INTEGRATION_DB"; tail -5 "$LOG_DIR/createdb.log"; exit 1; }
pass "created $INTEGRATION_DB"

note "== building all three services =="
(cd api && build cargo build --bin api > "$LOG_DIR/api-build.log" 2>&1) \
  || { fail "api build"; tail -20 "$LOG_DIR/api-build.log"; exit 1; }
(cd jobs && build cargo build > "$LOG_DIR/jobs-build.log" 2>&1) \
  || { fail "jobs build"; tail -20 "$LOG_DIR/jobs-build.log"; exit 1; }
(cd jobs && build cargo build --bin migrate > "$LOG_DIR/migrate-build.log" 2>&1) \
  || { fail "migrate build"; tail -20 "$LOG_DIR/migrate-build.log"; exit 1; }
(cd streaming && build cargo build > "$LOG_DIR/streaming-build.log" 2>&1) \
  || { fail "streaming build"; tail -20 "$LOG_DIR/streaming-build.log"; exit 1; }

MIGRATE_BIN=$(binary_of jobs migrate)
API_BIN=$(binary_of api api)
JOBS_BIN=$(binary_of jobs jobs)
STREAMING_BIN=$(binary_of streaming streaming)
for binary in "$MIGRATE_BIN" "$API_BIN" "$JOBS_BIN" "$STREAMING_BIN"; do
  [ -x "$binary" ] || { fail "binary not found at $binary"; exit 1; }
done

if (cd jobs && "$MIGRATE_BIN" all > "$LOG_DIR/migrate.log" 2>&1); then
  pass "one migrator carried both schemas into $INTEGRATION_DB"
else
  fail "migrations failed"
  tail -20 "$LOG_DIR/migrate.log"
  exit 1
fi

note "== seeding a listener, a session and one public track =="
psql_db -c "
  INSERT INTO soundcloud_connections (
      id, soundcloud_user_id, access_token, refresh_token, expires_at, scope
  ) VALUES (
      '$CONNECTION_ID', '900000001',
      'integration-access', 'integration-refresh', now() + interval '1 day', ''
  );
  INSERT INTO sessions (id, soundcloud_connection_id)
  VALUES ('$SESSION_ID', '$CONNECTION_ID');
  INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, storage_state)
  VALUES ('$SEEDED_TRACK', 'soundcloud:tracks:$SEEDED_TRACK', 'Integration Track',
          'integration track', 180000, 'public', 'ok');
" > "$LOG_DIR/seed.log" 2>&1 || { fail "seeding"; tail -10 "$LOG_DIR/seed.log"; exit 1; }
pass "seeded"

start_api() {
  port_is_free "$API_PORT" "api" || return 1
  (cd api && PORT="$API_PORT" ADMIN_TOKEN="$ADMIN_TOKEN" exec "$API_BIN" >> "$LOG_DIR/api.log" 2>&1) &
  api_pid="$!"
  wait_for "http://127.0.0.1:$API_PORT/health" "api health" "$api_pid"
}

start_jobs() {
  port_is_free "$JOBS_HEALTH_PORT" "jobs" || return 1
  (cd jobs && JOBS_HEALTH_BIND="127.0.0.1:$JOBS_HEALTH_PORT" exec "$JOBS_BIN" >> "$LOG_DIR/jobs.log" 2>&1) &
  jobs_pid="$!"
  wait_for "http://127.0.0.1:$JOBS_HEALTH_PORT/ready" "jobs readiness" "$jobs_pid"
}

stop_jobs() {
  [ -n "$jobs_pid" ] || return 0
  kill "$jobs_pid" 2>/dev/null
  wait "$jobs_pid" 2>/dev/null
  jobs_pid=""
}

start_streaming() {
  port_is_free "$STREAMING_PORT" "streaming" || return 1
  (cd streaming && PORT="$STREAMING_PORT" \
    DATABASE_HOST=127.0.0.1 DATABASE_PORT=55433 DATABASE_USERNAME=soundcloud \
    DATABASE_PASSWORD=devpw DATABASE_NAME="$INTEGRATION_DB" \
    exec "$STREAMING_BIN" >> "$LOG_DIR/streaming.log" 2>&1) &
  streaming_pid="$!"
  wait_for "http://127.0.0.1:$STREAMING_PORT/health" "streaming health" "$streaming_pid"
}

note "== all three services against one database =="
start_api || { tail -20 "$LOG_DIR/api.log"; exit 1; }
start_jobs || { tail -20 "$LOG_DIR/jobs.log"; exit 1; }
start_streaming || { tail -20 "$LOG_DIR/streaming.log"; exit 1; }

note "== the ticket api mints is the ticket streaming accepts =="
redirect=$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' --max-time 10 \
  -H "x-session-id: $SESSION_ID" \
  "http://127.0.0.1:$API_PORT/tracks/soundcloud:tracks:$SEEDED_TRACK/stream")
redirect_code=${redirect%% *}
redirect_url=${redirect##* }
if [ "$redirect_code" = "307" ] && [[ "$redirect_url" == *"127.0.0.1:$STREAMING_PORT/stream/"*"ticket="* ]]; then
  pass "api redirects the listener to streaming with a ticket"
else
  fail "api stream redirect -> $redirect_code $redirect_url"
fi

ticket=${redirect_url##*ticket=}
if [ -n "$ticket" ] && [ "$ticket" != "$redirect_url" ]; then
  accepted=$(status_of "http://127.0.0.1:$STREAMING_PORT/stream/soundcloud:tracks:$SEEDED_TRACK?ticket=$ticket")
  if [ "$accepted" = "401" ]; then
    fail "streaming refused the ticket api minted for it -> 401"
  else
    pass "streaming accepted api's ticket -> $accepted"
  fi

  last=${ticket: -1}
  forged="${ticket%?}$([ "$last" = "A" ] && printf B || printf A)"
  refused=$(status_of "http://127.0.0.1:$STREAMING_PORT/stream/soundcloud:tracks:$SEEDED_TRACK?ticket=$forged")
  if [ "$refused" = "401" ]; then
    pass "streaming refuses a ticket whose last character was changed -> 401"
  else
    fail "a tampered ticket was not refused -> $refused: the acceptance above proves nothing"
  fi

  other=$(status_of "http://127.0.0.1:$STREAMING_PORT/stream/soundcloud:tracks:$CLAIMED_TRACK?ticket=$ticket")
  if [ "$other" = "401" ]; then
    pass "a ticket bound to one track opens no other -> 401"
  else
    fail "a ticket for track $SEEDED_TRACK opened track $CLAIMED_TRACK -> $other"
  fi
else
  fail "no ticket to check: $redirect_url"
fi

note "== streaming metrics live behind the internal token =="
no_token=$(status_of "http://127.0.0.1:$STREAMING_PORT/metrics")
[ "$no_token" = "401" ] && pass "streaming /metrics without a token -> 401" \
  || fail "streaming /metrics without a token -> $no_token"
code=$(curl -s -o "$LOG_DIR/streaming-metrics.txt" -w '%{http_code}' --max-time 10 \
  -H "authorization: Bearer $INTERNAL_TOKEN" "http://127.0.0.1:$STREAMING_PORT/metrics")
if [ "$code" = "200" ] && grep -q "streaming_pg_pool_connections" "$LOG_DIR/streaming-metrics.txt"; then
  pass "streaming /metrics reports its pool against the shared database"
else
  fail "streaming /metrics -> $code without the pool gauge"
fi

accepted_enqueue() {
  psql_db -c "SELECT count(*) FROM background_job_enqueues WHERE id = '$1'"
}
queued_refresh() {
  psql_db -c "SELECT count(*) FROM background_jobs
              WHERE kind = 'catalog.refresh' AND payload->'payload'->>'sc_id' = '$1'"
}
touched_refresh() {
  psql_db -c "SELECT CASE
                WHEN EXISTS (SELECT 1 FROM background_jobs
                             WHERE kind = 'catalog.refresh'
                               AND payload->'payload'->>'sc_id' = '$1'
                               AND attempts > 0) THEN 1
                WHEN NOT EXISTS (SELECT 1 FROM background_jobs
                                 WHERE kind = 'catalog.refresh'
                                   AND payload->'payload'->>'sc_id' = '$1') THEN 1
                ELSE 0 END"
}

await() {
  local probe=$1 argument=$2 want=$3 deadline=$((SECONDS + INGRESS_SECONDS)) seen
  while [ "$SECONDS" -lt "$deadline" ]; do
    seen=$("$probe" "$argument")
    [ "${seen:-0}" -ge "$want" ] && return 0
    sleep 1
  done
  return 1
}

ask_for_aggregates() {
  curl -s --max-time 15 -X POST -H "x-admin-token: $ADMIN_TOKEN" \
    "http://127.0.0.1:$API_PORT/admin/discover/refresh"
}
job_id_of() { printf '%s' "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("jobId",""))' 2>/dev/null; }
cold_miss() {
  status_of -H "x-session-id: $SESSION_ID" \
    "http://127.0.0.1:$API_PORT/tracks/soundcloud:tracks:$1"
}

note "== what api publishes over NATS is what jobs writes down =="
answer=$(ask_for_aggregates)
live_job=$(job_id_of "$answer")
[ -n "$live_job" ] && pass "admin refresh accepted, job $live_job" \
  || fail "admin refresh gave no job id: $answer"
if [ -n "$live_job" ] && await accepted_enqueue "$live_job" 1; then
  pass "jobs recorded that exact message within ${INGRESS_SECONDS}s"
else
  fail "jobs never recorded the message api published"
  tail -10 "$LOG_DIR/jobs.log"
fi

note "== what api writes itself, jobs is the one that picks up =="
miss=$(cold_miss "$CLAIMED_TRACK")
[ "$miss" = "503" ] && pass "a track nobody has ever seen answers 503, not 500" \
  || fail "cold miss -> $miss, expected 503"
if [ "$(queued_refresh "$CLAIMED_TRACK")" = "1" ]; then
  pass "api recorded the refresh it promised"
else
  fail "api answered 503 without recording a refresh for $CLAIMED_TRACK"
fi
if await touched_refresh "$CLAIMED_TRACK" 1; then
  pass "jobs worked that row: it carries an attempt or has already left the queue"
else
  fail "the row sat untouched for ${INGRESS_SECONDS}s: nothing claimed it"
  tail -10 "$LOG_DIR/jobs.log"
fi

note "== with jobs stopped, neither half of that happens =="
stop_jobs
pass "jobs stopped"
answer=$(ask_for_aggregates)
orphan_job=$(job_id_of "$answer")
[ -n "$orphan_job" ] && pass "api still publishes with jobs down, job $orphan_job" \
  || fail "admin refresh with jobs down gave no job id: $answer"
miss=$(cold_miss "$UNCLAIMED_TRACK")
[ "$miss" = "503" ] && pass "api still answers 503 with jobs down" \
  || fail "cold miss with jobs down -> $miss, expected 503"
[ "$(queued_refresh "$UNCLAIMED_TRACK")" = "1" ] \
  && pass "api recorded the refresh with jobs down too" \
  || fail "no refresh row for $UNCLAIMED_TRACK to watch"
sleep "$INGRESS_SECONDS"
if [ "$(accepted_enqueue "$orphan_job")" = "0" ]; then
  pass "nothing wrote the published message down: the ingress check measured jobs, not api"
else
  fail "the message was recorded with jobs down: the ingress check proves nothing"
fi
if [ "$(touched_refresh "$UNCLAIMED_TRACK")" = "0" ]; then
  pass "nothing touched the waiting row: the claim check measured jobs, not the queue"
else
  fail "the row moved with jobs down: the claim check proves nothing"
fi

note "== jobs returns and drains what piled up while it was away =="
start_jobs || { tail -20 "$LOG_DIR/jobs.log"; exit 1; }
if await accepted_enqueue "$orphan_job" 1; then
  pass "the message published during the outage survived it"
else
  fail "the message published during the outage was lost"
  tail -10 "$LOG_DIR/jobs.log"
fi
if await touched_refresh "$UNCLAIMED_TRACK" 1; then
  pass "the row that waited out the outage was worked after it"
else
  fail "the row that waited out the outage is still untouched"
fi

note "== queue at the end =="
note "     pending=$(psql_db -c 'SELECT count(*) FROM background_jobs')" \
     "attempted=$(psql_db -c 'SELECT count(*) FROM background_jobs WHERE attempts > 0')" \
     "dead=$(psql_db -c 'SELECT count(*) FROM background_job_failures')"

note "== logs in $LOG_DIR =="
if [ "$failures" -eq 0 ]; then
  note "integration: ok"
  exit 0
fi
note "integration: $failures failed"
exit 1
