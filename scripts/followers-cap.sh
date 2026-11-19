#!/usr/bin/env bash
set -euo pipefail

: "${PSQL:?PSQL must run psql on core, e.g. PSQL='docker exec core-db-1 psql -U soundcloud -d soundcloud_desktop'}"
CAP="${CAP:-2000}"
CHUNK="${CHUNK:-2000}"
PAUSE="${PAUSE:-2}"
MAX_LAG_BYTES="${MAX_LAG_BYTES:-67108864}"
APPLY="${APPLY:-0}"
USERS_LIMIT="${USERS_LIMIT:-0}"
QUIET_HOURS="${QUIET_HOURS:-00 01 02}"

sql() {
  $PSQL -AtqX -v ON_ERROR_STOP=1 -c "SET statement_timeout = '300s'; $1" < /dev/null
}

wait_for_quiet_hours() {
  while [[ " $QUIET_HOURS " == *" $(date -u +%H) "* ]]; do
    echo "$(date -u +%FT%TZ) quiet hour, waiting"
    sleep 300
  done
}

wait_for_subscribers() {
  local lag
  wait_for_quiet_hours
  while :; do
    lag=$(sql "SELECT coalesce(max(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)), 0)::bigint
               FROM pg_replication_slots WHERE slot_type = 'logical' AND active")
    [ "$lag" -le "$MAX_LAG_BYTES" ] && return
    echo "$(date -u +%FT%TZ) waiting, subscriber lag $lag bytes"
    sleep 10
  done
}

open_walk="EXISTS (SELECT 1 FROM catalog_collection_sync
                   WHERE subject_id = f.user_id AND collection = 'followers' AND NOT complete)"

users=$(sql "SELECT f.user_id || ' ' || count(*) FROM user_followers f
             GROUP BY f.user_id HAVING count(*) > $CAP
             ORDER BY count(*) DESC
             LIMIT nullif($USERS_LIMIT, 0)")
echo "$(date -u +%FT%TZ) accounts over $CAP followers: $(grep -c . <<< "$users" || true)," \
  "rows over the cap: $(awk -v cap="$CAP" '{ s += $2 - cap } END { print s + 0 }' <<< "$users")"
[ "$APPLY" = 1 ] || { echo "dry run; APPLY=1 deletes"; exit 0; }

done_users=0
skipped=0
rows=0
while read -r user count; do
  [ -n "${user:-}" ] || continue
  [[ "$user" =~ ^[0-9]+$ ]] || { echo "skip user $user"; continue; }
  if [ "$(sql "SELECT count(*) FROM (SELECT '$user' AS user_id) f WHERE $open_walk")" != 0 ]; then
    echo "$(date -u +%FT%TZ) skip $user ($count rows), its walk is open"
    skipped=$((skipped + 1))
    continue
  fi
  while :; do
    wait_for_subscribers
    n=$(sql "WITH doomed AS (
                 SELECT f.target_user_urn FROM user_followers f
                 WHERE f.user_id = '$user' AND NOT $open_walk
                 ORDER BY f.created_at DESC, f.target_user_urn DESC
                 OFFSET $CAP LIMIT $CHUNK
             ), gone AS (
                 DELETE FROM user_followers AS follower USING doomed
                 WHERE follower.user_id = '$user' AND follower.target_user_urn = doomed.target_user_urn
                 RETURNING 1
             )
             SELECT count(*) FROM gone")
    rows=$((rows + n))
    [ "$n" -lt "$CHUNK" ] && break
    sleep "$PAUSE"
  done
  done_users=$((done_users + 1))
  if [ $((done_users % 20)) -eq 0 ]; then
    echo "$(date -u +%FT%TZ) accounts $done_users, rows $rows"
  fi
done <<< "$users"
echo "$(date -u +%FT%TZ) done: accounts $done_users, skipped open $skipped, rows $rows"
