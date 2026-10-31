#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repl="$(cd "$here/../.." && pwd)"
root="$(cd "$repl/../.." && pwd)"
tmp="$(mktemp -d)"
. "$here/lib.sh"

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
val postgres "SELECT count(*) FROM pg_catalog.pg_roles WHERE rolname = 'scd_repl'" | grep -q 1 || sql -d postgres -c "CREATE ROLE scd_repl NOLOGIN"
for n in $nodes; do
  fresh "scd_b1_$n" scd_b1_tpl
  role=api; [ "$n" = core ] && role=core
  val "scd_b1_$n" "SELECT string_agg(x, '; ') FROM public.scd_configure('$n', '$role', 'split') x" > /dev/null
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM public.scd_configure('$n', '$role', 'split')")" = 0 ]
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM public.scd_repl_problems()")" -gt 0 ]
  [ "$n" = core ] && weak_grant "scd_b1_$n"
  val "scd_b1_$n" "SELECT count(*) FROM public.scd_grant_repl()" > /dev/null
  problems="$(val "scd_b1_$n" "SELECT string_agg(x, '; ') FROM public.scd_repl_problems() x")"
  [ -z "$problems" ] || { echo "FAIL scd_repl problems on $n: $problems"; exit 1; }
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM public.scd_sync_publications()")" -gt 0 ]
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM public.scd_sync_publications()")" = 0 ]
done
echo "ok scd_configure, scd_grant_repl and scd_sync_publications are idempotent; scd_repl_problems is empty after the grant"
for n in $nodes; do
  bad="$(val "scd_b1_$n" "SELECT count(*) FROM public.scd_publication_lists() l
    JOIN pg_catalog.pg_publication_tables p ON p.pubname = l.pubname AND p.schemaname = 'public' AND p.tablename = l.tbl
    WHERE l.node = CASE public.scd_role() WHEN 'core' THEN 'core' ELSE 'api' END
      AND ARRAY(SELECT c::text FROM unnest(p.attnames) c ORDER BY 1) <> public.scd_publication_columns(l.tbl, l.cols)")"
  [ "$bad" = 0 ] || { echo "FAIL publication column lists differ on $n: $bad"; exit 1; }
done
echo "ok publication column lists = planned columns that exist"

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
wait_for scd_b1_main "SELECT progress FROM public.user_likes_tracks WHERE user_id = 'r1'" "core's result reaches the api node"
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'r1'"
wait_for scd_b1_core "SELECT NOT wanted_state AND progress FROM public.user_likes_tracks WHERE user_id = 'r1'" "a later intent keeps core's result"
wait_for scd_b1_star "SELECT NOT wanted_state AND progress FROM public.user_likes_tracks WHERE user_id = 'r1'" "and reaches the other api node"
pairs user_likes_tracks "user_id = 'r1'" "M row equal on all nodes"

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
queued="$(val scd_b1_core "SELECT string_agg(tbl || ' ' || (pk - 'v')::text, ', ') FROM public.repl_resend")"
[ "$queued" = 'repl_tombstones {"pk": {"id": "00000000-0000-0000-0000-0000000000d2"}, "tbl": "disliked_tracks"}' ] \
  || { echo "FAIL core should queue only the tombstone back toward star, queued: $queued"; exit 1; }
[ "$(val scd_b1_core "SELECT public.scd_drain_resend()")" = 1 ]
echo "ok core queues the tombstone back toward star and drains it"

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
[ "$(val scd_b1_core "SELECT count(*) FILTER (WHERE public.scd_purge_intent('user_likes_tracks', jsonb_build_object('user_id', user_id, 'sc_track_id', sc_track_id), wanted_v)) FROM (SELECT user_id, sc_track_id, wanted_v FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3') ORDER BY 1) x")" = 2 ]
[ "$(val scd_b1_core "SELECT count(*) FROM public.repl_purges WHERE tbl = 'user_likes_tracks' AND pk ->> 'user_id' IN ('r2', 'r3')")" = 2 ]
if val scd_b1_main "SELECT public.scd_purge_intent('user_likes_tracks', '{\"user_id\": \"r3\", \"sc_track_id\": \"t1\"}', 0)" 2>/dev/null; then
  echo "FAIL scd_purge_intent ran on an api node"; exit 1
fi
echo "ok scd_purge_intent writes the receipt and deletes in one tx on core, refuses on an api node"
wait_for scd_b1_star "SELECT count(*) = 0 FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3')" "star applies both purges"
wait_for scd_b1_main "SELECT array_agg(user_id) = ARRAY['r3'] FROM public.user_likes_tracks WHERE user_id IN ('r2', 'r3')" "main applies the covered purge and keeps its newer intent"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE pk = '{\"user_id\": \"r3\", \"sc_track_id\": \"t1\"}')" "main queues the kept row for resend"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_main ENABLE"

. "$here/split.sh"
. "$here/converge.sh"
. "$here/verify.sh"
. "$here/retention.sh"
. "$here/s1.sh"
. "$here/hold.sh"

say "all subscriptions still running"
for n in $nodes; do
  wait_for "scd_b1_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "no apply worker died on $n"
done
errors="$(val postgres "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats")"
[ "$errors" = 0 ] || { echo "FAIL apply errors: $errors"; exit 1; }
echo "ok no apply errors"
say "all checks passed"
