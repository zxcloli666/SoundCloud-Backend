#!/usr/bin/env bash
set -euo pipefail

: "${PSQL:?PSQL must run psql on core, e.g. PSQL='docker exec core-db-1 psql -U soundcloud -d soundcloud_desktop'}"
CAP="${CAP:-2000}"
SUBJECTS="${SUBJECTS:-20}"
PAUSE="${PAUSE:-1}"
MAX_LAG_BYTES="${MAX_LAG_BYTES:-67108864}"
QUIET_HOURS="${QUIET_HOURS:-00 01 02}"
PASS="${PASS:-copy}"

sql() {
  $PSQL -AtqX -v ON_ERROR_STOP=1 -c "SET statement_timeout = '120s'; $1" < /dev/null
}

wait_for_calm() {
  local lag
  while [[ " $QUIET_HOURS " == *" $(date -u +%H) "* ]]; do
    echo "$(date -u +%FT%TZ) quiet hour, waiting"
    sleep 300
  done
  while :; do
    lag=$(sql "SELECT coalesce(max(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)), 0)::bigint
               FROM pg_replication_slots WHERE slot_type = 'logical' AND active")
    [ "$lag" -le "$MAX_LAG_BYTES" ] && return
    echo "$(date -u +%FT%TZ) waiting, subscriber lag $lag bytes"
    sleep 10
  done
}

batch_sql() {
  cat <<SQL
WITH subjects AS MATERIALIZED (
    SELECT c.subject_id, c.collection,
           CASE c.collection WHEN 'track-favoriters' THEN 1 WHEN 'track-reposters' THEN 2 ELSE 3 END::smallint AS relation,
           CASE c.collection WHEN 'playlist-reposters' THEN 'soundcloud:playlists:' ELSE 'soundcloud:tracks:' END || c.subject_id AS urn
    FROM catalog_collection_sync c
    WHERE c.collection IN ('track-favoriters', 'track-reposters', 'playlist-reposters')
      AND c.scope = 'public'
      AND (c.subject_id, c.collection) > ('$1', '$2')
    ORDER BY c.subject_id, c.collection
    LIMIT $SUBJECTS
), wanted AS MATERIALIZED (
    SELECT s.subject_id::bigint AS subject_id, s.relation,
           substring(a.user_urn FROM '^soundcloud:users:([0-9]{1,18})\$')::bigint AS user_id,
           a.created_at
    FROM subjects s
    CROSS JOIN LATERAL (
        SELECT user_urn, created_at FROM catalog_audience
        WHERE subject_urn = s.urn AND relation = s.collection
        ORDER BY created_at DESC, user_urn DESC
        LIMIT $CAP
    ) a
    WHERE s.subject_id ~ '^[0-9]{1,18}\$' AND a.user_urn ~ '^soundcloud:users:[0-9]{1,18}\$'
), copied AS (
    INSERT INTO catalog_audience_ids (subject_id, relation, user_id, created_at)
    SELECT subject_id, relation, user_id, created_at FROM wanted
    ON CONFLICT (subject_id, relation, user_id) DO NOTHING
    RETURNING 1
), extra AS (
    DELETE FROM catalog_audience_ids ids
    USING subjects s
    WHERE '$PASS' = 'check'
      AND s.subject_id ~ '^[0-9]{1,18}\$'
      AND ids.subject_id = s.subject_id::bigint AND ids.relation = s.relation
      AND NOT EXISTS (
          SELECT 1 FROM wanted w
          WHERE w.subject_id = ids.subject_id AND w.relation = ids.relation AND w.user_id = ids.user_id
      )
      AND NOT EXISTS (
          SELECT 1 FROM catalog_audience a
          WHERE a.subject_urn = s.urn AND a.relation = s.collection
            AND a.user_urn = 'soundcloud:users:' || ids.user_id
      )
    RETURNING 1
)
SELECT coalesce((SELECT subject_id || ' ' || collection FROM subjects ORDER BY subject_id DESC, collection DESC LIMIT 1), ''),
       (SELECT count(*) FROM subjects),
       (SELECT count(*) FROM copied),
       (SELECT count(*) FROM extra)
SQL
}

case "$PASS" in copy|check) ;; *) echo "PASS is copy or check"; exit 1 ;; esac
sql "SELECT count(*) || ' audience subjects to ' || '$PASS' FROM catalog_collection_sync
     WHERE collection IN ('track-favoriters', 'track-reposters', 'playlist-reposters') AND scope = 'public'"

last_subject=""
last_collection=""
subjects=0
copied=0
extra=0
batches=0
while :; do
  wait_for_calm
  out=$(sql "$(batch_sql "$last_subject" "$last_collection")")
  IFS='|' read -r last n c e <<< "$out"
  [ "${n:-0}" -gt 0 ] || break
  read -r last_subject last_collection <<< "$last"
  [[ "$last_subject $last_collection" =~ ^[0-9A-Za-z_:-]+\ [a-z-]+$ ]] || { echo "odd cursor: $last"; exit 1; }
  subjects=$((subjects + n))
  copied=$((copied + c))
  extra=$((extra + e))
  batches=$((batches + 1))
  if [ $((batches % 50)) -eq 0 ]; then
    echo "$(date -u +%FT%TZ) subjects $subjects, copied $copied, extra deleted $extra, at $last_subject $last_collection"
  fi
  sleep "$PAUSE"
done
echo "$(date -u +%FT%TZ) $PASS done: subjects $subjects, copied $copied, extra deleted $extra"
