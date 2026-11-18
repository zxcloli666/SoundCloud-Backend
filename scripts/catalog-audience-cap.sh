#!/usr/bin/env bash
set -euo pipefail

: "${PSQL:?PSQL must run psql on core, e.g. PSQL='docker exec -i core-db-1 psql -U scd -d scd'}"
CAP="${CAP:-2000}"
CHUNK="${CHUNK:-5000}"
PAUSE="${PAUSE:-1}"
MAX_LAG_BYTES="${MAX_LAG_BYTES:-67108864}"
APPLY="${APPLY:-0}"
AUDIENCE="'track-favoriters', 'track-reposters', 'playlist-reposters'"

sql() {
  $PSQL -AtqX -v ON_ERROR_STOP=1 -c "SET statement_timeout = '120s'; $1"
}

wait_for_subscribers() {
  local lag
  while :; do
    lag=$(sql "SELECT coalesce(max(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)), 0)::bigint
               FROM pg_replication_slots WHERE slot_type = 'logical' AND active")
    [ "$lag" -le "$MAX_LAG_BYTES" ] && return
    echo "$(date -u +%FT%TZ) waiting, subscriber lag $lag bytes"
    sleep 10
  done
}

subject_urn() {
  case "$1" in
    playlist-reposters) echo "soundcloud:playlists:$2" ;;
    *) echo "soundcloud:tracks:$2" ;;
  esac
}

open=$(sql "SELECT count(*) FROM catalog_collection_sync
            WHERE collection IN ($AUDIENCE) AND scope = 'public' AND NOT complete AND item_count > $CAP")
echo "open walks over the cap: $open (left alone until they complete)"
sql "SELECT count(*) || ' subjects, ~' || coalesce(sum(item_count - $CAP), 0) || ' audience rows over the cap'
     FROM catalog_collection_sync
     WHERE collection IN ($AUDIENCE) AND scope = 'public' AND complete AND item_count > $CAP"
[ "$APPLY" = 1 ] || { echo "dry run; APPLY=1 deletes"; exit 0; }

subjects=$(sql "SELECT collection || ' ' || subject_id FROM catalog_collection_sync
                WHERE collection IN ($AUDIENCE) AND scope = 'public' AND complete AND item_count > $CAP
                ORDER BY item_count DESC")

done_subjects=0
audience_rows=0
seen_rows=0
while read -r collection subject; do
  [ -n "${subject:-}" ] || continue
  [[ "$subject" =~ ^[0-9]+$ ]] || { echo "skip subject $subject"; continue; }
  urn=$(subject_urn "$collection" "$subject")
  complete="EXISTS (SELECT 1 FROM catalog_collection_sync
                    WHERE subject_id = '$subject' AND collection = '$collection' AND scope = 'public' AND complete)"
  while :; do
    wait_for_subscribers
    n=$(sql "WITH doomed AS (
                 SELECT ctid FROM catalog_audience
                 WHERE subject_urn = '$urn' AND relation = '$collection' AND $complete
                 ORDER BY created_at DESC, user_urn DESC
                 OFFSET $CAP LIMIT $CHUNK
             ), gone AS (
                 DELETE FROM catalog_audience AS audience USING doomed
                 WHERE audience.ctid = doomed.ctid
                 RETURNING 1
             )
             SELECT count(*) FROM gone")
    audience_rows=$((audience_rows + n))
    [ "$n" -lt "$CHUNK" ] && break
    sleep "$PAUSE"
  done
  while :; do
    n=$(sql "WITH doomed AS (
                 SELECT seen.ctid FROM catalog_collection_seen AS seen
                 WHERE seen.subject_id = '$subject' AND seen.collection = '$collection' AND seen.scope = 'public'
                   AND $complete
                   AND NOT EXISTS (
                       SELECT 1 FROM catalog_audience AS audience
                       WHERE audience.subject_urn = '$urn' AND audience.relation = '$collection'
                         AND audience.user_urn = seen.entity_key
                   )
                 LIMIT $CHUNK
             ), gone AS (
                 DELETE FROM catalog_collection_seen AS seen USING doomed
                 WHERE seen.ctid = doomed.ctid
                 RETURNING 1
             )
             SELECT count(*) FROM gone")
    seen_rows=$((seen_rows + n))
    [ "$n" -lt "$CHUNK" ] && break
    sleep "$PAUSE"
  done
  done_subjects=$((done_subjects + 1))
  if [ $((done_subjects % 100)) -eq 0 ]; then
    echo "$(date -u +%FT%TZ) subjects $done_subjects, audience rows $audience_rows, seen rows $seen_rows"
  fi
done <<< "$subjects"
echo "$(date -u +%FT%TZ) done: subjects $done_subjects, audience rows $audience_rows, seen rows $seen_rows"
