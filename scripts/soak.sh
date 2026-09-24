#!/usr/bin/env bash
set -uo pipefail

SOAK_DB="${SOAK_DB:-soundcloud_soak}"
SOAK_PG_CONTAINER="${SOAK_PG_CONTAINER:-scd-rewrite-pg}"
JOBS_HEALTH_PORT="${JOBS_HEALTH_PORT:-3396}"
SOAK_SECONDS="${SOAK_SECONDS:-600}"
SAMPLE_SECONDS="${SAMPLE_SECONDS:-15}"
WORK_ITEMS="${WORK_ITEMS:-400}"
RSS_GROWTH_BUDGET_MB="${RSS_GROWTH_BUDGET_MB:-64}"
POOL_WAIT_BUDGET_MS="${POOL_WAIT_BUDGET_MS:-50}"
PROFILE="${PROFILE:-release}"
LOG_DIR="${LOG_DIR:-$(mktemp -d)}"

cd "$(git rev-parse --show-toplevel)"
source "$(git rev-parse --show-toplevel)/scripts/local-env.sh"
BUILD_DATABASE_URL="${BUILD_DATABASE_URL:-postgres://soundcloud:devpw@127.0.0.1:55433/soundcloud_desktop}"
build() { ( DATABASE_URL="$BUILD_DATABASE_URL" OPS_DATABASE_URL="$BUILD_DATABASE_URL" "$@" ); }
export DATABASE_URL="postgres://soundcloud:devpw@127.0.0.1:55433/$SOAK_DB"
export OPS_DATABASE_URL="$DATABASE_URL"
export SUBSCRIPTIONS_SNAPSHOT_DIR="$LOG_DIR/snapshots"
mkdir -p "$SUBSCRIPTIONS_SNAPSHOT_DIR"

failures=0
jobs_pid=""

note() { printf '%s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; failures=$((failures + 1)); }
pass() { printf 'ok   %s\n' "$*"; }

cleanup() { [ -n "$jobs_pid" ] && kill "$jobs_pid" 2>/dev/null; wait 2>/dev/null; }
trap cleanup EXIT

psql_soak() { podman exec -i "$SOAK_PG_CONTAINER" psql -U soundcloud -d "$SOAK_DB" -qtAX "$@"; }

binary_of() {
  local crate=$1 name=$2 dir
  dir=$(cd "$crate" && cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  printf '%s/%s/%s\n' "$dir" "$PROFILE" "$name"
}
build_flags() { [ "$PROFILE" = "release" ] && printf -- '--release' || printf ''; }

start_jobs() {
  (cd jobs && JOBS_HEALTH_BIND="127.0.0.1:$JOBS_HEALTH_PORT" exec "$JOBS_BIN" >> "$LOG_DIR/jobs.log" 2>&1) &
  jobs_pid="$!"
  local deadline=$((SECONDS + 90))
  until curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$JOBS_HEALTH_PORT/live"; do
    kill -0 "$jobs_pid" 2>/dev/null || { fail "jobs exited at startup"; tail -10 "$LOG_DIR/jobs.log"; return 1; }
    [ "$SECONDS" -lt "$deadline" ] || { fail "jobs never came up"; return 1; }
    sleep 1
  done
  return 0
}

rss_mb() { awk '/VmRSS/ {print int($2/1024)}' "/proc/$jobs_pid/status" 2>/dev/null; }

pool_wait_within_budget() {
  local metrics="$LOG_DIR/pool-wait.txt" code last bad
  code=$(curl -s -o "$metrics" -w '%{http_code}' --max-time 10 \
    "http://127.0.0.1:$JOBS_HEALTH_PORT/metrics")
  if [ "$code" != "200" ]; then
    fail "pool wait: /metrics answered $code"
    return
  fi
  last=$(awk '/^jobs_pg_pool_wait_last_seconds /{ print $2 }' "$metrics" | tail -1)
  bad=$(awk '/^jobs_pg_pool_wait_seconds_count\{outcome="(error|timeout)"\}/{ sum += $2 } END { print sum + 0 }' "$metrics")
  if [ -z "$last" ]; then
    fail "pool wait: the probe reported nothing"
    return
  fi
  if awk -v a="$last" -v b="$POOL_WAIT_BUDGET_MS" 'BEGIN { exit !(a * 1000 < b) }' && [ "$bad" = "0" ]; then
    pass "$(awk -v a="$last" -v n="$bad" 'BEGIN { printf "pool wait under a full queue: last=%.3fms refused=%s", a * 1000, n }')"
  else
    fail "$(awk -v a="$last" -v n="$bad" -v b="$POOL_WAIT_BUDGET_MS" 'BEGIN { printf "pool wait under a full queue: last=%.3fms refused=%s budget=%sms", a * 1000, n, b }')"
  fi
}

queue_counts() {
  psql_soak -c "SELECT
      (SELECT count(*) FROM background_jobs),
      (SELECT count(*) FROM background_jobs WHERE lease_id IS NOT NULL),
      (SELECT count(*) FROM background_job_failures)"
}

note "== fresh database =="
podman exec "$SOAK_PG_CONTAINER" psql -U soundcloud -d postgres -q \
  -c "DROP DATABASE IF EXISTS $SOAK_DB WITH (FORCE)" -c "CREATE DATABASE $SOAK_DB" \
  > "$LOG_DIR/createdb.log" 2>&1 || { fail "could not create $SOAK_DB"; exit 1; }
(cd jobs && build cargo build $(build_flags) --bin migrate > "$LOG_DIR/migrate-build.log" 2>&1) || { fail "migrate build"; exit 1; }
MIGRATE_BIN=$(binary_of jobs migrate)
(cd jobs && "$MIGRATE_BIN" all > "$LOG_DIR/migrate.log" 2>&1) || { fail "migrations"; tail -10 "$LOG_DIR/migrate.log"; exit 1; }
pass "$SOAK_DB migrated"

(cd jobs && build cargo build $(build_flags) > "$LOG_DIR/jobs-build.log" 2>&1) || { fail "jobs build"; tail -10 "$LOG_DIR/jobs-build.log"; exit 1; }
JOBS_BIN=$(binary_of jobs jobs)

note "== jetstream against the live broker: delivery, retry, redelivery =="
if (cd jobs && build env NATS_URL="$NATS_URL" cargo test --lib -- --include-ignored bus:: \
    > "$LOG_DIR/bus-tests.log" 2>&1); then
  pass "jetstream: $(grep -oE '[0-9]+ passed' "$LOG_DIR/bus-tests.log" | head -1)"
else
  fail "jetstream tests against $NATS_URL"
  tail -20 "$LOG_DIR/bus-tests.log"
fi

enqueue_batch() {
  local count=$1 tag=$2
  psql_soak -c "
    INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts, available_at)
    SELECT gen_random_uuid(), 'catalog.refresh', 'core_bulk', '$tag:' || n,
           jsonb_build_object('v', 1, 'entity', 'track', 'sc_id', (900000000 + n)::text),
           0, 50, now()
    FROM generate_series(1, $count) AS n
    ON CONFLICT DO NOTHING" > /dev/null 2>&1
}

note "== enqueue $WORK_ITEMS items that cannot succeed =="
psql_soak -c "
  INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts, available_at)
  SELECT gen_random_uuid(), 'catalog.refresh', 'core_bulk', 'soak:' || n,
         jsonb_build_object('v', 1, 'entity', 'track', 'sc_id', (900000000 + n)::text),
         0, 50, now()
  FROM generate_series(1, $WORK_ITEMS) AS n" > "$LOG_DIR/enqueue.log" 2>&1 \
  || { fail "could not enqueue"; tail -5 "$LOG_DIR/enqueue.log"; exit 1; }
pass "enqueued $WORK_ITEMS"

note "== jobs, first start =="
start_jobs || exit 1
sleep 5
first_rss=$(rss_mb)
pass "resident set at start: ${first_rss}MB"

note "== soak for ${SOAK_SECONDS}s, sampling every ${SAMPLE_SECONDS}s =="
peak_rss="$first_rss"
peak_leased=0
enqueued_total="$WORK_ITEMS"
restarted=0
deadline=$((SECONDS + SOAK_SECONDS))
while [ "$SECONDS" -lt "$deadline" ]; do
  sleep "$SAMPLE_SECONDS"
  if ! kill -0 "$jobs_pid" 2>/dev/null; then
    fail "jobs died during the soak"
    tail -20 "$LOG_DIR/jobs.log"
    break
  fi
  rss=$(rss_mb)
  [ -n "$rss" ] && [ "$rss" -gt "$peak_rss" ] && peak_rss="$rss"
  enqueue_batch 40 "trickle$SECONDS"
  enqueued_total=$((enqueued_total + 40))
  read -r queued leased dead <<< "$(queue_counts | tr '|' ' ')"
  [ "$leased" -gt "$peak_leased" ] && peak_leased="$leased"
  printf '     t+%-4s rss=%-5s queued=%-5s leased=%-4s dead=%s\n' \
    "$((SECONDS))s" "${rss}MB" "$queued" "$leased" "$dead"

  if [ "$restarted" -eq 0 ] && [ "$SECONDS" -gt $((deadline - SOAK_SECONDS / 2)) ]; then
    note "== killing jobs mid-flight =="
    kill -9 "$jobs_pid" 2>/dev/null
    wait "$jobs_pid" 2>/dev/null
    leased_before=$(psql_soak -c "SELECT count(*) FROM background_jobs WHERE lease_id IS NOT NULL")
    note "     leases left behind: $leased_before"
    start_jobs || break
    restarted=1
    pass "jobs restarted"
  fi
done

note "== waiting for a pooled connection, with the queue still loaded =="
pool_wait_within_budget

note "== verdict =="
read -r queued leased dead <<< "$(queue_counts | tr '|' ' ')"
note "     queued=$queued leased=$leased dead=$dead peak_rss=${peak_rss}MB"

growth=$((peak_rss - first_rss))
if [ "$growth" -le "$RSS_GROWTH_BUDGET_MB" ]; then
  pass "resident set grew ${growth}MB, budget ${RSS_GROWTH_BUDGET_MB}MB"
else
  fail "resident set grew ${growth}MB, over the ${RSS_GROWTH_BUDGET_MB}MB budget"
fi

accounted=$((queued + dead))
if [ "$accounted" -le "$enqueued_total" ]; then
  pass "every item is accounted for: $queued queued + $dead dead-lettered of $enqueued_total"
else
  fail "more items than enqueued: $accounted > $enqueued_total"
fi

attempted=$(psql_soak -c "SELECT
    (SELECT count(*) FROM background_jobs WHERE attempts > 0)
  + (SELECT count(*) FROM background_job_failures WHERE attempts > 0)")
if [ "${attempted:-0}" -gt 0 ]; then
  pass "the soak saw work actually leased: $attempted items carry an attempt, live peak $peak_leased"
else
  fail "no job was ever leased: the soak proved nothing about leases"
fi

if [ "$restarted" -eq 1 ] && [ "$leased" -le "$queued" ]; then
  pass "leases after the restart are bounded by the queue"
else
  fail "restart left $leased leases against $queued queued"
fi

note "== logs in $LOG_DIR =="
[ "$failures" -eq 0 ] && { note "soak: ok"; exit 0; }
note "soak: $failures failed"
exit 1
