#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repl="$(cd "$here/../.." && pwd)"
root="$(cd "$repl/../.." && pwd)"
psql_cmd="${PSQL:-psql}"
socket="${PG_SOCKET:-/var/run/postgresql}"
nodes="core main star"
tmp="$(mktemp -d)"

sql() { $psql_cmd -X -q -v ON_ERROR_STOP=1 "$@"; }
val() { $psql_cmd -X -q -v ON_ERROR_STOP=1 -At -d "$1" -c "$2"; }
say() { printf '== %s\n' "$*"; }

teardown() {
  for n in $nodes; do
    for s in $(val postgres "SELECT s.subname FROM pg_catalog.pg_subscription s JOIN pg_catalog.pg_database d ON d.oid = s.subdbid WHERE d.datname = 'scd_b1_$n'"); do
      sql -d "scd_b1_$n" -c "ALTER SUBSCRIPTION $s DISABLE" -c "ALTER SUBSCRIPTION $s SET (slot_name = NONE)" -c "DROP SUBSCRIPTION $s"
    done
  done
  for s in $(val postgres "SELECT slot_name FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd_b1_%'"); do
    val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
  done
  for d in scd_b1_core scd_b1_main scd_b1_star scd_b1_smoke scd_b1_exercise scd_b1_tpl scd_b1_ops; do
    sql -d postgres -c "DROP DATABASE IF EXISTS $d WITH (FORCE)"
  done
}

fresh() {
  sql -d postgres -c "SET client_min_messages = warning" -c "DROP DATABASE IF EXISTS $1 WITH (FORCE)" -c "CREATE DATABASE $1 ${2:+TEMPLATE $2}"
}

install() {
  for f in "$repl/generated/tables.sql" "$here/b2_stub.sql" "$repl/generated/triggers.sql" \
           "$repl/generated/row_hash.sql" "$repl/generated/purge_children.sql" "$repl/generated/publications.sql"; do
    sql -d "$1" -c "SET search_path = ''" -c "SET client_min_messages = warning" -f - < "$f"
  done
}

wait_for() {
  local i
  for i in $(seq 1 120); do
    if [ "$(val "$1" "$2")" = t ]; then
      echo "ok $3"
      return 0
    fi
    sleep 0.5
  done
  echo "FAIL $3 (on $1: $2)"
  exit 1
}

mark() {
  sql -d "scd_b1_$1" -c "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight) VALUES ('00000000-0000-0000-0000-00000000$2', 'mark', 'mark', 'mark', 0)"
}

same_on_all() {
  local a b c
  a="$(val scd_b1_core "$1")"; b="$(val scd_b1_main "$1")"; c="$(val scd_b1_star "$1")"
  if [ -z "$a" ] || [ "$a" != "$b" ] || [ "$a" != "$c" ]; then
    echo "FAIL $2: core=$a main=$b star=$c"
    exit 1
  fi
  echo "ok $2"
}

trap 'teardown > /dev/null 2>&1 || true; rm -rf "$tmp"' EXIT
teardown

say "migrate a template database"
fresh scd_b1_tpl
fresh scd_b1_ops
for f in "$root"/api/migrations/*.sql; do sql -d scd_b1_tpl -c "SET client_min_messages = error" -f - < "$f"; done
for f in "$root"/api/migrations-ops/*.sql; do sql -d scd_b1_ops -c "SET client_min_messages = error" -f - < "$f"; done

say "schema.json matches the migrations"
PSQL="$psql_cmd" SCHEMA_OUT="$tmp/schema.json" "$repl/schema.sh" scd_b1_tpl scd_b1_ops
if [ "${SCHEMA_UPDATE:-}" = 1 ]; then
  cp "$tmp/schema.json" "$repl/schema.json"
  echo "schema.json updated"
fi
diff -u "$repl/schema.json" "$tmp/schema.json"
echo "ok schema.json"

say "install generated SQL twice with an empty search_path"
install scd_b1_tpl
sql -d scd_b1_tpl -c "CREATE TRIGGER zz_ver BEFORE UPDATE ON public.albums FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger()"
install scd_b1_tpl
[ "$(val scd_b1_tpl "SELECT count(*) FROM pg_catalog.pg_trigger WHERE tgrelid = 'public.albums'::regclass AND tgname = 'zz_ver'")" = 0 ]
echo "ok install is idempotent and drops a stale generated trigger"

say "smoke per trigger class"
fresh scd_b1_smoke scd_b1_tpl
sql -At -d scd_b1_smoke -f - < "$here/smoke.sql" > "$tmp/smoke.log" 2>&1 || { cat "$tmp/smoke.log"; exit 1; }
sed -n 's/.*NOTICE:  \(ok .*\)/\1/p' "$tmp/smoke.log"

say "every generated function runs against the real schema"
fresh scd_b1_exercise scd_b1_tpl
sql -At -d scd_b1_exercise -f - < "$here/exercise.sql" > "$tmp/exercise.log" 2>&1 || { cat "$tmp/exercise.log"; exit 1; }
sed -n 's/.*NOTICE:  \(ok .*\)/\1/p' "$tmp/exercise.log"

say "three nodes on real logical replication"
for n in $nodes; do
  fresh "scd_b1_$n" scd_b1_tpl
  role=api; [ "$n" = core ] && role=core
  sql -d postgres -c "ALTER DATABASE scd_b1_$n SET scd.role = '$role'" -c "ALTER DATABASE scd_b1_$n SET scd.node = '$n'" \
    -c "ALTER DATABASE scd_b1_$n SET scd.mode = 'split'"
  sql -d "scd_b1_$n" -f - <<SQL
DO \$d\$
DECLARE r record;
BEGIN
    FOR r IN SELECT DISTINCT pubname, publish FROM public.scd_publication_lists() WHERE node = '$role' ORDER BY 1 LOOP
        EXECUTE format('CREATE PUBLICATION %I WITH (publish = %L)', r.pubname, r.publish);
    END LOOP;
    FOR r IN SELECT * FROM public.scd_publication_lists() WHERE node = '$role' ORDER BY 1, 4 LOOP
        EXECUTE format('ALTER PUBLICATION %I ADD TABLE public.%I (%s)', r.pubname, r.tbl,
            (SELECT string_agg(quote_ident(c), ', ' ORDER BY c) FROM unnest(r.cols) c));
    END LOOP;
END
\$d\$;
SQL
done

subscribe() {
  val "scd_b1_$2" "SELECT pg_catalog.pg_create_logical_replication_slot('scd_b1_$1_from_$2', 'pgoutput')" > /dev/null
  sql -d "scd_b1_$1" -c "CREATE SUBSCRIPTION $1_from_$2 CONNECTION 'host=$socket dbname=scd_b1_$2 user=scd' PUBLICATION $3 WITH ($4, create_slot = false, slot_name = 'scd_b1_$1_from_$2', copy_data = false)"
}
subscribe core main "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe core star "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe main core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
subscribe star core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
for n in $nodes; do
  wait_for "scd_b1_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up on $n"
done

say "M: intent from an api node, result from core, per-group merge"
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('r1', 't1')"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'r1')" "an api insert reaches the other api node through core"
sql -d scd_b1_core -c "UPDATE public.user_likes_tracks SET progress = true, synced_at = '2026-01-01' WHERE user_id = 'r1'"
wait_for scd_b1_main "SELECT progress AND result_v % 16 = 1 FROM public.user_likes_tracks WHERE user_id = 'r1'" "core's result reaches the api node"
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'r1'"
wait_for scd_b1_core "SELECT NOT wanted_state AND progress FROM public.user_likes_tracks WHERE user_id = 'r1'" "a later intent keeps core's result"
wait_for scd_b1_star "SELECT NOT wanted_state AND progress FROM public.user_likes_tracks WHERE user_id = 'r1'" "and reaches the other api node"
same_on_all "SELECT public.scd_row_hash('user_likes_tracks', '{\"user_id\": \"r1\", \"sc_track_id\": \"t1\"}') || ' ' || wanted_v || ' ' || result_v FROM public.user_likes_tracks WHERE user_id = 'r1'" "M row, hash and versions equal on all nodes"

say "A: the same R6 id written on two api nodes converges to the newer version"
sql -d scd_b1_star -c "ALTER SUBSCRIPTION star_from_core DISABLE"
wait_for scd_b1_star "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'star_from_core' AND pid IS NOT NULL)" "star stops reading core"
sql -d scd_b1_main -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-0000000000d1', 'r1', 'x1')"
sql -d scd_b1_star -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-0000000000d1', 'r1', 'x1')"
sql -d scd_b1_star -c "ALTER SUBSCRIPTION star_from_core ENABLE"
wait_for scd_b1_main "SELECT row_v % 16 = 3 FROM public.disliked_tracks WHERE sc_user_id = 'r1'" "main takes star's newer row"
wait_for scd_b1_core "SELECT row_v % 16 = 3 FROM public.disliked_tracks WHERE sc_user_id = 'r1'" "core keeps star's newer row"
same_on_all "SELECT id || ' ' || row_v FROM public.disliked_tracks WHERE sc_user_id = 'r1'" "A row equal on all nodes"

say "A: a tombstone blocks a late insert"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_star DISABLE"
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'core_from_star' AND pid IS NOT NULL)" "core stops reading star"
sql -d scd_b1_star -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-0000000000d2', 'r1', 'x2')"
mark star ee01
sql -d scd_b1_main -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-0000000000d2', 'r1', 'x2')"
sql -d scd_b1_main -c "DELETE FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d2'"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.repl_tombstones WHERE tbl = 'disliked_tracks') AND NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_track_id = 'x2')" "core has main's delete and tombstone"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_star ENABLE"
wait_for scd_b1_star "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_track_id = 'x2') AND EXISTS (SELECT 1 FROM public.repl_tombstones WHERE tbl = 'disliked_tracks')" "star applies the delete and the tombstone"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000ee01')" "core consumed star's late insert (marker behind it arrived)"
same_on_all "SELECT count(*) FROM public.disliked_tracks WHERE sc_track_id = 'x2'" "the late insert stays deleted on all nodes (0 rows)"

say "C: core wins a unique key over an optimistic api row"
sql -d scd_b1_main -c "SET scd.cmd = '00000000-0000-0000-0000-000000000001'" \
  -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000a1', 'Opt', 'rn', 'sc')"
sql -d scd_b1_core -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000a2', 'Core', 'rn', 'sc')"
wait_for scd_b1_main "SELECT array_agg(id::text) = ARRAY['00000000-0000-0000-0000-0000000000a2'] FROM public.artists WHERE normalized_name = 'rn'" "main purged its optimistic row and took core's"
sql -d scd_b1_main -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000a3', 'Raw', 'raw', 'sc')"
wait_for scd_b1_main "SELECT n = 1 FROM public.scd_counters WHERE counter = 'guard' AND tbl = 'artists'" "the guard counts the unwrapped api write"
wait_for scd_b1_core "SELECT n = 0 FROM public.scd_counters WHERE counter = 'guard' AND tbl = 'artists'" "core's own C write is not counted"

say "K: core keeps the max register"
sql -d scd_b1_main -c "INSERT INTO public.sc_track_counters (sc_track_id, fetched_at) VALUES ('k1', '2026-01-02')"
wait_for scd_b1_core "SELECT fetched_at = '2026-01-02' FROM public.sc_track_counters WHERE sc_track_id = 'k1'" "core has main's register"
sql -d scd_b1_star -c "INSERT INTO public.sc_track_counters (sc_track_id, fetched_at) VALUES ('k1', '2026-01-01')"
mark star ee02
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000ee02')" "core consumed star's older register (marker behind it arrived)"
wait_for scd_b1_core "SELECT fetched_at = '2026-01-02' FROM public.sc_track_counters WHERE sc_track_id = 'k1'" "core keeps the newer register"

say "M: a core purge applies where the receipt covers the intent and is refused where it does not"
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('r2', 't1'), ('r3', 't1')"
wait_for scd_b1_star "SELECT count(*) = 2 FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3')" "two likes on all nodes"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_main DISABLE"
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'core_from_main' AND pid IS NOT NULL)" "core stops reading main"
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'r3'"
sql -d scd_b1_core -f - <<'SQL'
BEGIN;
INSERT INTO public.repl_purges (tbl, pk, purged_v)
SELECT 'user_likes_tracks', jsonb_build_object('user_id', user_id, 'sc_track_id', sc_track_id), wanted_v
FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3');
DELETE FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3');
COMMIT;
SQL
wait_for scd_b1_star "SELECT count(*) = 0 FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3')" "star applies both purges"
wait_for scd_b1_main "SELECT array_agg(user_id) = ARRAY['r3'] FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3')" "main applies the covered purge and keeps its newer intent"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE pk = '{\"user_id\": \"r3\", \"sc_track_id\": \"t1\"}')" "main queues the kept row for resend"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_main ENABLE"

say "all subscriptions still running"
for n in $nodes; do
  wait_for "scd_b1_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "no apply worker died on $n"
done
errors="$(val postgres "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats")"
[ "$errors" = 0 ] || { echo "FAIL apply errors: $errors"; exit 1; }
echo "ok no apply errors"
say "all checks passed"
