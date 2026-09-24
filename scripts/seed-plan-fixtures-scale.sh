#!/usr/bin/env bash
set -euo pipefail
: "${PSQL:?PSQL required, e.g. PSQL='podman exec -i scd-planfixture-big psql -U soundcloud -d soundcloud_planfixture_big'}"
PROD_LIKE='{"users":100000,"artists":1200000,"tracks":6600000,"track_artists":3400000,"sc_track_counters":1800000,"user_likes_tracks":1400000,"artist_colike":3000000,"user_events":130000000}'
OVERRIDES="${OVERRIDES:-$PROD_LIKE}"

cd "$(git rev-parse --show-toplevel)"

{
  printf "SET planfixture.overrides = %s;\n" "'$OVERRIDES'"
  cat <<'SQL'
SET maintenance_work_mem = '1GB';
SET synchronous_commit = off;
BEGIN;
CREATE TEMP TABLE planfixture_indexes AS
SELECT i.indexname, i.tablename, i.indexdef
  FROM pg_indexes AS i
  JOIN pg_class AS c ON c.relname = i.indexname AND c.relnamespace = 'public'::regnamespace
 WHERE i.schemaname = 'public'
   AND i.tablename IN (SELECT jsonb_object_keys(current_setting('planfixture.overrides')::jsonb))
   AND NOT EXISTS (SELECT 1 FROM pg_constraint AS k WHERE k.conindid = c.oid);
DO
$$
DECLARE
    dropped text;
BEGIN
    FOR dropped IN SELECT indexname FROM planfixture_indexes LOOP
        EXECUTE format('DROP INDEX %I', dropped);
    END LOOP;
END
$$;
COMMIT;
SQL
  cat scripts/seed-plan-fixtures.sql
  cat <<'SQL'
SELECT indexdef FROM planfixture_indexes ORDER BY tablename, indexname
\gexec
VACUUM (ANALYZE);
SELECT relname, n_live_tup, pg_size_pretty(pg_total_relation_size(relid)) AS total
  FROM pg_stat_user_tables
 WHERE relname IN (SELECT jsonb_object_keys(current_setting('planfixture.overrides')::jsonb))
 ORDER BY n_live_tup DESC;
SQL
} | $PSQL -v ON_ERROR_STOP=1
