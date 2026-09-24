#!/usr/bin/env bash
set -uo pipefail

API_PORT="${API_PORT:-3387}"
JOBS_HEALTH_PORT="${JOBS_HEALTH_PORT:-3388}"
ADMIN_TOKEN="${ADMIN_TOKEN:-chaos-token}"
WAIT_SECONDS="${WAIT_SECONDS:-90}"
DOWN_SECONDS="${DOWN_SECONDS:-8}"
SETTLE_SECONDS="${SETTLE_SECONDS:-20}"
BASELINE_SECONDS="${BASELINE_SECONDS:-8}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

cd "$(git rev-parse --show-toplevel)"

CHAOS_DB="${CHAOS_DB:-soundcloud_chaos}"
CHAOS_PG_CONTAINER="${CHAOS_PG_CONTAINER:-scd-chaos-pg}"
CHAOS_PG_PORT="${CHAOS_PG_PORT:-55435}"
CHAOS_PG_DATA="${CHAOS_PG_DATA:-/mnt/steam-games/scd-pg/chaos}"
CHAOS_NATS_CONTAINER="${CHAOS_NATS_CONTAINER:-scd-chaos-nats}"
CHAOS_NATS_PORT="${CHAOS_NATS_PORT:-4226}"
CHAOS_REDIS_CONTAINER="${CHAOS_REDIS_CONTAINER:-scd-chaos-redis}"
CHAOS_REDIS_PORT="${CHAOS_REDIS_PORT:-6380}"
CHAOS_QDRANT_CONTAINER="${CHAOS_QDRANT_CONTAINER:-scd-qdrant-test}"
KEEP_STANDS="${KEEP_STANDS:-false}"
TRAFFIC_ROUTE="${TRAFFIC_ROUTE:-/me}"

source "$(git rev-parse --show-toplevel)/scripts/local-env.sh"
BUILD_DATABASE_URL="${BUILD_DATABASE_URL:-postgres://soundcloud:devpw@127.0.0.1:55433/soundcloud_desktop}"
build() { ( DATABASE_URL="$BUILD_DATABASE_URL" OPS_DATABASE_URL="$BUILD_DATABASE_URL" "$@" ); }
export DATABASE_URL="postgres://soundcloud:devpw@127.0.0.1:$CHAOS_PG_PORT/$CHAOS_DB"
export OPS_DATABASE_URL="$DATABASE_URL"
export NATS_URL="nats://127.0.0.1:$CHAOS_NATS_PORT"
export REDIS_URL="redis://127.0.0.1:$CHAOS_REDIS_PORT"
export SUBSCRIPTIONS_SNAPSHOT_DIR="$LOG_DIR/snapshots"
mkdir -p "$SUBSCRIPTIONS_SNAPSHOT_DIR"

SESSION_ID="33333333-3333-4333-8333-333333333333"
CONNECTION_ID="44444444-4444-4444-8444-444444444444"
SEEDED_TRACK="800000001"

failures=0
api_pid=""
jobs_pid=""
traffic_pid=""
TRAFFIC_LOG="$LOG_DIR/traffic.log"

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }

cleanup() {
  [ -n "$traffic_pid" ] && kill "$traffic_pid" 2>/dev/null
  for pid in "$api_pid" "$jobs_pid"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null
  done
  wait 2>/dev/null
  podman start "$CHAOS_QDRANT_CONTAINER" > /dev/null 2>&1
  if [ "$KEEP_STANDS" != "true" ]; then
    podman rm -f "$CHAOS_NATS_CONTAINER" "$CHAOS_REDIS_CONTAINER" > /dev/null 2>&1
    podman stop -t 5 "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
  fi
}
trap cleanup EXIT

psql_db() { podman exec -i "$CHAOS_PG_CONTAINER" psql -U soundcloud -d "$CHAOS_DB" -qtAX "$@" < /dev/null; }

binary_of() {
  local crate=$1 name=$2 dir
  dir=$(cd "$crate" && cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  printf '%s/debug/%s\n' "$dir" "$name"
}

port_answers() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }

await_port() {
  local port=$1 name=$2 deadline=$((SECONDS + 60))
  until port_answers "$port"; do
    [ "$SECONDS" -lt "$deadline" ] || { fail "$name never listened on $port"; return 1; }
    sleep 1
  done
  return 0
}

note "== stands of our own, so nothing else feels this =="
podman rm -f "$CHAOS_NATS_CONTAINER" "$CHAOS_REDIS_CONTAINER" > /dev/null 2>&1
podman run -d --name "$CHAOS_NATS_CONTAINER" -p "127.0.0.1:$CHAOS_NATS_PORT:4222" \
  docker.io/library/nats:2-alpine -js > "$LOG_DIR/nats.log" 2>&1 \
  || { fail "could not start $CHAOS_NATS_CONTAINER"; exit 1; }
podman run -d --name "$CHAOS_REDIS_CONTAINER" -p "127.0.0.1:$CHAOS_REDIS_PORT:6379" \
  docker.io/library/redis:7-alpine > "$LOG_DIR/redis.log" 2>&1 \
  || { fail "could not start $CHAOS_REDIS_CONTAINER"; exit 1; }

if ! podman container exists "$CHAOS_PG_CONTAINER" 2>/dev/null; then
  mkdir -p "$CHAOS_PG_DATA"
  podman run -d --name "$CHAOS_PG_CONTAINER" \
    -e POSTGRES_USER=soundcloud -e POSTGRES_PASSWORD=devpw -e POSTGRES_DB="$CHAOS_DB" \
    -v "$CHAOS_PG_DATA:/var/lib/postgresql/data" \
    -p "127.0.0.1:$CHAOS_PG_PORT:5432" \
    docker.io/library/postgres:17-alpine -c max_connections=100 \
    > "$LOG_DIR/pg.log" 2>&1 || { fail "could not start $CHAOS_PG_CONTAINER"; exit 1; }
else
  podman start "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
fi
await_port "$CHAOS_NATS_PORT" "$CHAOS_NATS_CONTAINER" || exit 1
await_port "$CHAOS_REDIS_PORT" "$CHAOS_REDIS_CONTAINER" || exit 1
await_port "$CHAOS_PG_PORT" "$CHAOS_PG_CONTAINER" || exit 1
deadline=$((SECONDS + 60))
until podman exec "$CHAOS_PG_CONTAINER" pg_isready -U soundcloud -q < /dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || { fail "$CHAOS_PG_CONTAINER never became ready"; exit 1; }
  sleep 1
done
pass "postgres $CHAOS_PG_PORT, redis $CHAOS_REDIS_PORT, nats $CHAOS_NATS_PORT"

note "== fresh database =="
podman exec "$CHAOS_PG_CONTAINER" psql -U soundcloud -d postgres -q \
  -c "DROP DATABASE IF EXISTS $CHAOS_DB WITH (FORCE)" -c "CREATE DATABASE $CHAOS_DB" \
  > "$LOG_DIR/createdb.log" 2>&1 || { fail "could not create $CHAOS_DB"; tail -5 "$LOG_DIR/createdb.log"; exit 1; }

note "== building =="
(cd api && build cargo build --bin api > "$LOG_DIR/api-build.log" 2>&1) \
  || { fail "api build"; tail -20 "$LOG_DIR/api-build.log"; exit 1; }
(cd jobs && build cargo build > "$LOG_DIR/jobs-build.log" 2>&1) \
  || { fail "jobs build"; tail -20 "$LOG_DIR/jobs-build.log"; exit 1; }
(cd jobs && build cargo build --bin migrate > "$LOG_DIR/migrate-build.log" 2>&1) \
  || { fail "migrate build"; tail -20 "$LOG_DIR/migrate-build.log"; exit 1; }
MIGRATE_BIN=$(binary_of jobs migrate)
API_BIN=$(binary_of api api)
JOBS_BIN=$(binary_of jobs jobs)
(cd jobs && "$MIGRATE_BIN" all > "$LOG_DIR/migrate.log" 2>&1) \
  || { fail "migrations"; tail -20 "$LOG_DIR/migrate.log"; exit 1; }
pass "migrated"

psql_db -c "
  INSERT INTO soundcloud_connections (
      id, soundcloud_user_id, access_token, refresh_token, expires_at, scope
  ) VALUES (
      '$CONNECTION_ID', '900000002',
      'chaos-access', 'chaos-refresh', now() + interval '1 day', ''
  );
  INSERT INTO sessions (id, soundcloud_connection_id) VALUES ('$SESSION_ID', '$CONNECTION_ID');
  INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, storage_state)
  VALUES ('$SEEDED_TRACK', 'soundcloud:tracks:$SEEDED_TRACK', 'Chaos Track',
          'chaos track', 180000, 'public', 'ok');
" > "$LOG_DIR/seed.log" 2>&1 || { fail "seeding"; tail -10 "$LOG_DIR/seed.log"; exit 1; }
pass "seeded"

start_api() {
  (cd api && PORT="$API_PORT" ADMIN_TOKEN="$ADMIN_TOKEN" exec "$API_BIN" >> "$LOG_DIR/api.log" 2>&1) &
  api_pid="$!"
  local deadline=$((SECONDS + WAIT_SECONDS))
  until curl -fs -o /dev/null --max-time 2 "http://127.0.0.1:$API_PORT/health" 2>/dev/null; do
    kill -0 "$api_pid" 2>/dev/null || { fail "api exited at startup"; tail -20 "$LOG_DIR/api.log"; return 1; }
    [ "$SECONDS" -lt "$deadline" ] || { fail "api never answered"; return 1; }
    sleep 1
  done
  return 0
}

start_jobs() {
  (cd jobs && JOBS_HEALTH_BIND="127.0.0.1:$JOBS_HEALTH_PORT" exec "$JOBS_BIN" >> "$LOG_DIR/jobs.log" 2>&1) &
  jobs_pid="$!"
  local deadline=$((SECONDS + WAIT_SECONDS))
  until curl -fs -o /dev/null --max-time 2 "http://127.0.0.1:$JOBS_HEALTH_PORT/ready" 2>/dev/null; do
    kill -0 "$jobs_pid" 2>/dev/null || { fail "jobs exited at startup"; tail -20 "$LOG_DIR/jobs.log"; return 1; }
    [ "$SECONDS" -lt "$deadline" ] || { fail "jobs never became ready"; return 1; }
    sleep 1
  done
  return 0
}

note "== api and jobs up =="
start_api || exit 1
start_jobs || exit 1
pass "both services answer"

: > "$TRAFFIC_LOG"
(
  while true; do
    curl -s -o /dev/null -w '%{http_code}\n' --max-time 5 \
      -H "x-session-id: $SESSION_ID" \
      "http://127.0.0.1:$API_PORT$TRAFFIC_ROUTE" >> "$TRAFFIC_LOG" 2>/dev/null \
      || printf '000\n' >> "$TRAFFIC_LOG"
    sleep 0.2
  done
) &
traffic_pid="$!"

mark() { wc -l < "$TRAFFIC_LOG" | tr -d ' '; }
slice_since() { tail -n "+$(( $1 + 1 ))" "$TRAFFIC_LOG"; }
ok_in() { slice_since "$1" | grep -c '^200$'; }
bad_in() { slice_since "$1" | grep -vc '^200$'; }
api_alive() { kill -0 "$api_pid" 2>/dev/null; }

note "== baseline for ${BASELINE_SECONDS}s =="
start=$(mark)
sleep "$BASELINE_SECONDS"
good=$(ok_in "$start"); bad=$(bad_in "$start")
if [ "$good" -gt 0 ] && [ "$bad" -eq 0 ]; then
  pass "baseline: $good answered, $bad refused"
else
  fail "baseline: $good answered, $bad refused — the run starts from a broken state"
fi

note "== postgres taken away for ${DOWN_SECONDS}s while listeners keep asking =="
outage=$(mark)
podman stop -t 5 "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
sleep "$DOWN_SECONDS"
felt=$(bad_in "$outage")
served=$(ok_in "$outage")
podman start "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
deadline=$((SECONDS + 60))
until podman exec "$CHAOS_PG_CONTAINER" pg_isready -U soundcloud -q < /dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || { fail "postgres never came back"; break; }
  sleep 1
done
if [ "$felt" -gt "$served" ]; then
  pass "the outage was real: $felt of $((felt + served)) requests were refused while postgres was down"
else
  fail "$TRAFFIC_ROUTE answered $served of $((felt + served)) requests with no database: it does not read postgres, so the recovery below would prove nothing"
fi
if api_alive; then
  pass "api survived losing its database"
else
  fail "api exited when postgres went away"
  tail -20 "$LOG_DIR/api.log"
fi
sleep "$SETTLE_SECONDS"
settled=$(mark)
sleep "$BASELINE_SECONDS"
good=$(ok_in "$settled"); bad=$(bad_in "$settled")
if [ "$good" -gt 0 ] && [ "$bad" -eq 0 ]; then
  pass "api reconnected on its own: $good answered, $bad refused, no restart"
else
  fail "after postgres returned: $good answered, $bad refused"
fi

note "== redis taken away: the cache must not be load-bearing =="
outage=$(mark)
podman stop -t 5 "$CHAOS_REDIS_CONTAINER" > /dev/null 2>&1
if port_answers "$CHAOS_REDIS_PORT"; then
  fail "redis still answers on $CHAOS_REDIS_PORT: the outage below is imaginary"
else
  pass "redis is really gone"
fi
sleep "$DOWN_SECONDS"
good=$(ok_in "$outage"); bad=$(bad_in "$outage")
if [ "$good" -gt 0 ] && [ "$bad" -eq 0 ]; then
  pass "serving carried on without redis: $good answered, $bad refused"
else
  fail "losing redis cost $bad requests"
fi
podman start "$CHAOS_REDIS_CONTAINER" > /dev/null 2>&1
await_port "$CHAOS_REDIS_PORT" "$CHAOS_REDIS_CONTAINER"

note "== nats taken away: reads must not wait on the broker =="
outage=$(mark)
podman stop -t 5 "$CHAOS_NATS_CONTAINER" > /dev/null 2>&1
sleep "$DOWN_SECONDS"
good=$(ok_in "$outage"); bad=$(bad_in "$outage")
if [ "$good" -gt 0 ] && [ "$bad" -eq 0 ]; then
  pass "serving carried on without nats: $good answered, $bad refused"
else
  fail "losing nats cost $bad requests"
fi
spent=$(curl -s -o /dev/null -w '%{http_code} %{time_total}' --max-time 20 \
  -X POST -H "x-admin-token: $ADMIN_TOKEN" \
  "http://127.0.0.1:$API_PORT/admin/discover/refresh")
publish_code=${spent%% *}
publish_time=${spent##* }
if [ "$publish_code" != "200" ] && awk -v t="$publish_time" 'BEGIN { exit !(t < 5) }'; then
  pass "publishing with no broker gave up in ${publish_time}s with $publish_code instead of hanging"
else
  fail "publishing with no broker -> $publish_code after ${publish_time}s"
fi
podman start "$CHAOS_NATS_CONTAINER" > /dev/null 2>&1
await_port "$CHAOS_NATS_PORT" "$CHAOS_NATS_CONTAINER"
sleep "$SETTLE_SECONDS"
publish_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 \
  -X POST -H "x-admin-token: $ADMIN_TOKEN" "http://127.0.0.1:$API_PORT/admin/discover/refresh")
if [ "$publish_code" = "200" ]; then
  pass "publishing works again once the broker is back -> 200"
else
  fail "publishing after the broker returned -> $publish_code"
fi

note "== the vector store taken away: plain serving must not care =="
outage=$(mark)
podman stop -t 10 "$CHAOS_QDRANT_CONTAINER" > /dev/null 2>&1
sleep "$DOWN_SECONDS"
good=$(ok_in "$outage"); bad=$(bad_in "$outage")
if [ "$good" -gt 0 ] && [ "$bad" -eq 0 ]; then
  pass "serving carried on without qdrant: $good answered, $bad refused"
else
  fail "losing qdrant cost $bad requests"
fi
if api_alive; then
  pass "api survived losing the vector store"
else
  fail "api exited when qdrant went away"
fi
podman start "$CHAOS_QDRANT_CONTAINER" > /dev/null 2>&1
await_port 6334 "$CHAOS_QDRANT_CONTAINER"

note "== jobs through a database outage =="
psql_db -c "
  INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts, available_at)
  SELECT gen_random_uuid(), 'catalog.refresh', 'core_bulk', 'chaos:' || n,
         jsonb_build_object('version', '1', 'payload',
             jsonb_build_object('entity', 'track', 'sc_id', (880000000 + n)::text)),
         0, 50, now()
  FROM generate_series(1, 60) AS n" > /dev/null 2>&1
attempted_before=$(psql_db -c "SELECT count(*) FROM background_jobs WHERE attempts > 0")
podman stop -t 5 "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
sleep "$DOWN_SECONDS"
jobs_alive_during=$(kill -0 "$jobs_pid" 2>/dev/null && printf yes || printf no)
podman start "$CHAOS_PG_CONTAINER" > /dev/null 2>&1
deadline=$((SECONDS + 60))
until podman exec "$CHAOS_PG_CONTAINER" pg_isready -U soundcloud -q < /dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || { fail "postgres never came back for jobs"; break; }
  sleep 1
done
if [ "$jobs_alive_during" = "yes" ]; then
  pass "jobs stayed up while its database was gone"
else
  fail "jobs exited when postgres went away"
  tail -20 "$LOG_DIR/jobs.log"
fi
sleep "$SETTLE_SECONDS"
attempted_after=$(psql_db -c "SELECT count(*) FROM background_jobs WHERE attempts > 0")
if kill -0 "$jobs_pid" 2>/dev/null && [ "${attempted_after:-0}" -gt "${attempted_before:-0}" ]; then
  pass "jobs went back to work after the outage: attempted $attempted_before -> $attempted_after"
else
  fail "jobs did not take work after the outage: attempted $attempted_before -> $attempted_after"
  tail -20 "$LOG_DIR/jobs.log"
fi

kill "$traffic_pid" 2>/dev/null
traffic_pid=""
total=$(wc -l < "$TRAFFIC_LOG" | tr -d ' ')
note "== $total requests sent over the whole run, logs in $LOG_DIR =="

if [ "$failures" -eq 0 ]; then
  note "chaos: ok"
  exit 0
fi
note "chaos: $failures failed"
exit 1
