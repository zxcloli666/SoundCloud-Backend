sweep_all() {
  local x n=0
  for x in $nodes; do n=$((n + $(val "scd_b1_$x" "SELECT sent + skipped + gone FROM public.scd_sweep()"))); done
  echo "$n"
}

swept() {
  local round
  for round in 1 2 3 4 5 6 7 8; do
    calm
    sweeps="$round"
    [ "$(sweep_all)" = 0 ] && return 0
  done
  echo "FAIL resend queues still not empty after 8 sweep rounds"
  exit 1
}

digests() {
  val "scd_b1_$1" "SELECT v.t || ' ' || c.n || ' ' || c.h FROM (VALUES $2) v (t, s) CROSS JOIN LATERAL public.scd_verify_chunk(v.t, NULL, NULL, v.s) c ORDER BY 1"
}

drift() {
  local shown
  shown="$(val scd_b1_core "SELECT string_agg(format('(%L, %L::text[])', t, public.scd_verify_shown(t)), ', ') FROM public.scd_verify_tables() t")"
  digests core "$shown" > "$tmp/dg.core"
  digests "$1" "$shown" > "$tmp/dg.$1"
  { diff "$tmp/dg.core" "$tmp/dg.$1" || true; } | sed -n 's/^> \([^ ]*\) .*/\1/p' | tr '\n' ' ' | sed 's/ $//'
}

only_on() {
  val "scd_b1_$1" "SELECT count(*) FROM public.$2 WHERE $3"
}

reach() {
  local c out
  mk=$((mk + 1)); c="$(printf '%04x' "$mk")"
  mark core "$c"
  out="$(for n in main star; do wait_for "scd_b1_$n" "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$c')" "$n applied core's stream"; done)" || { echo "$out"; exit 1; }
}

verify_cycle() {
  local dry="$1" out="$2" round rep
  : > "$out"
  : > "$out.recheck"
  val scd_b1_core "SELECT state || ' ' || probes || ' ' || wrapped FROM public.scd_verify_probe(100000, ${3:-500})" > "$tmp/probe.out"
  for round in 1 2 3 4 5 6 7 8 9 10; do
    calm
    sweep_all > /dev/null
    calm
    rep="$(val scd_b1_core "SELECT peer || ' ' || rel || ' ' || coalesce(k::text, '-') || ' ' || action || ' ' || attempt FROM public.scd_verify_repair($dry, 1000)")"
    [ -n "$rep" ] && printf '%s\n' "$rep" | awk -v out="$out" '$(NF-1) == "recheck" {print >> (out ".recheck"); next} {print >> out}'
    if [ -z "$rep" ] && [ "$(val scd_b1_core "SELECT count(*) FROM public.repl_checksums WHERE node <> 'core'")" = 0 ] \
      && [ "$(val scd_b1_main "SELECT count(*) FROM public.repl_resend")" = 0 ] && [ "$(val scd_b1_star "SELECT count(*) FROM public.repl_resend")" = 0 ]; then
      return 0
    fi
  done
  echo "FAIL verify did not settle in 10 rounds"
  cat "$out"
  exit 1
}

has() {
  grep -q -- "$2" "$1" && echo "ok $3" || { echo "FAIL $3: no line with $2 in"; cat "$1"; failed=$((failed + 1)); }
}

say "B13 sweep: legacy, busy, counts"
[ "$(val scd_b1_tpl "SELECT state || ' ' || sent || ' ' || queued FROM public.scd_sweep()")" = "legacy 0 0" ] \
  && echo "ok legacy mode does nothing" || { echo "FAIL legacy sweep: $(val scd_b1_tpl "SELECT row_to_json(s) FROM public.scd_sweep() s")"; exit 1; }
sql -d scd_b1_main -c "SET application_name = 'b13_lock'" -c "BEGIN" -c "SELECT pg_advisory_xact_lock(hashtextextended('scd.sweep', 0))" -c "SELECT pg_sleep(3)" -c "COMMIT" > /dev/null &
locker=$!
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'b13_lock' AND wait_event = 'PgSleep')" "a second runner holds the sweep lock"
[ "$(val scd_b1_main "SELECT state FROM public.scd_sweep()")" = busy ] && echo "ok one runner per node: the second sweep is busy" || { echo "FAIL the sweep ran beside a holder of its lock"; exit 1; }
wait "$locker"
quiet
dislike main sw1 1
dislike main sw2 1
calm
sql -d scd_b1_main -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'sw1', 'x'))), ('disliked_tracks', jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'sw2', 'x'))), ('disliked_tracks', jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'swgone', 'x')))"
sql -d scd_b1_main -c "SET application_name = 'b13_row'" -c "BEGIN" -c "SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'sw2' FOR UPDATE" -c "SELECT pg_sleep(3)" -c "COMMIT" > /dev/null &
locker=$!
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'b13_row' AND wait_event = 'PgSleep')" "a writer holds sw2"
got="$(val scd_b1_main "SELECT state || ' sent=' || sent || ' skipped=' || skipped || ' gone=' || gone || ' queued=' || queued FROM public.scd_sweep()")"
[ "$got" = "ran sent=1 skipped=1 gone=1 queued=1" ] && echo "ok sweep counts: $got" || { echo "FAIL sweep counts: $got"; exit 1; }
wait "$locker"
got="$(val scd_b1_main "SELECT state || ' sent=' || sent || ' skipped=' || skipped || ' gone=' || gone || ' queued=' || queued FROM public.scd_sweep()")"
[ "$got" = "ran sent=1 skipped=0 gone=0 queued=0" ] && echo "ok the skipped key goes on the next sweep: $got" || { echo "FAIL second sweep: $got"; exit 1; }
got="$(val scd_b1_main "SELECT state || ' sent=' || sent || ' skipped=' || skipped || ' gone=' || gone || ' queued=' || queued FROM public.scd_sweep()")"
[ "$got" = "ran sent=0 skipped=0 gone=0 queued=0" ] && echo "ok an empty queue: $got" || { echo "FAIL empty sweep: $got"; exit 1; }

say "B13 sweep: the lab T7 W3 shape converges with scd_sweep alone"
for u in w4a w4b w4c; do dislike main "$u" 1; done
calm
pause star star_from_core
for u in w4a w4b w4c; do undislike main "$u"; done
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id LIKE 'w4%')" "core applies main's deletes"
sql -d scd_b1_star -c "UPDATE public.disliked_tracks SET track_data = track_data || '{\"late\": 1}' WHERE sc_user_id LIKE 'w4%'"
resume star star_from_core
calm
[ "$(val scd_b1_star "SELECT count(*) FROM public.repl_resend WHERE tbl = 'disliked_tracks'")" = 3 ] || { echo "FAIL star did not queue its newer rows"; exit 1; }
swept
expect "after $sweeps sweep rounds star's newer rows live on every node" \
  "SELECT count(*) FILTER (WHERE track_data ->> 'late' = '1') FROM public.disliked_tracks WHERE sc_user_id LIKE 'w4%'" 3

say "B13 verify: the different-id R6 rows go, so whole-table digests compare"
for n in $nodes; do
  sql -d "scd_b1_$n" -c "SET session_replication_role = replica" -c "DELETE FROM public.disliked_tracks WHERE sc_track_id = 'other'"
done
swept
[ "$(drift main) / $(drift star)" = "artists / " ] && echo "ok before B13 only the unwrapped C write of main differs (artists a3)" \
  || { echo "FAIL drift before the B13 cases: main=$(drift main) star=$(drift star)"; exit 1; }

say "B13 verify: an api C write that core never had is dropped"
verify_cycle false "$tmp/verify.raw"
has "$tmp/verify.raw" 'main artists {"id": "00000000-0000-0000-0000-0000000000a3"} drop 1' "repair drops main's unwrapped C row"
[ "$(wc -l < "$tmp/verify.raw" | tr -d ' ')" = 1 ] && echo "ok nothing else is reported" || { echo "FAIL extra reports"; cat "$tmp/verify.raw"; failed=$((failed + 1)); }
swept
for n in main star; do
  d="$(drift "$n")"
  [ -z "$d" ] && echo "ok core and $n agree on every verified table" || { echo "FAIL core and $n differ: $d"; exit 1; }
done
probe_sent="$(val scd_b1_core "SELECT state || ' ' || probes || ' ' || wrapped FROM public.scd_verify_probe(100000, 500)")"
reach
calm
sweep_all > /dev/null
calm
[ "$(val scd_b1_core "SELECT count(*) FROM public.repl_checksums WHERE node <> 'core'")" = 0 ] \
  && echo "ok a full probe cycle on equal nodes gets no answer ($probe_sent)" || { echo "FAIL equal nodes answered a probe"; exit 1; }
[ "$(val scd_b1_main "SELECT state || ' ' || probes FROM public.scd_verify_probe()")" = "idle 0" ] && echo "ok probes run on core only" || { echo "FAIL main probed"; exit 1; }

say "B13 verify: core loses its slot to main; C deletes, M purges and core-deleted A rows of the gap stay on main"
art() { echo "00000000-0000-0000-0000-0000000013a$1"; }
for i in 1 2 3 4; do
  sql -d scd_b1_core -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('$(art $i)', 'Gap $i', 'gap$i', 'sc')"
done
sql -d scd_b1_core -c "INSERT INTO public.diag_events (id, app_version, body, fingerprint, install, kind, os, title) VALUES ('00000000-0000-0000-0000-0000000013d1', '1', '{}', 'fp', 'in', 'k', 'os', 'gap')"
for u in vm1 vm2; do sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('$u', 't1')"; done
dislike main vt1 1
calm
for u in vm1 vm2; do sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = '$u'"; done
calm
vm_v="$(val scd_b1_core "SELECT string_agg(user_id || '=' || wanted_v, ' ' ORDER BY user_id) FROM public.user_likes_tracks WHERE user_id IN ('vm1', 'vm2') AND NOT wanted_state")"
case "$vm_v" in "vm1="*" vm2="*) ;; *) echo "FAIL core did not see both unlikes: $vm_v"; exit 1 ;; esac
pause main main_from_core
dislike star ct1 1
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'ct1')" "core has star's row of the gap"
dislike main ct1 2
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'ct1' AND track_data ->> 'v' = '2')" "core merges main's insert of the same key, main never saw star's"
sql -d scd_b1_core -c "DELETE FROM public.artists WHERE id IN ('$(art 1)', '$(art 4)')" \
  -c "UPDATE public.artists SET name = 'Gap 2 renamed' WHERE id = '$(art 2)'" \
  -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('$(art 5)', 'Gap 5', 'gap5', 'sc')" \
  -c "DELETE FROM public.diag_events WHERE id = '00000000-0000-0000-0000-0000000013d1'"
for kv in $vm_v; do
  u="${kv%%=*}"; v="${kv##*=}"
  [ "$(val scd_b1_core "SELECT public.scd_purge_intent('user_likes_tracks', jsonb_build_object('user_id', '$u', 'sc_track_id', 't1'), $v)")" = t ] || { echo "FAIL core did not purge $u"; exit 1; }
done
undislike core vt1
wait_for scd_b1_star "SELECT NOT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id IN ('vm1', 'vm2'))" "star applies the purges"
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = true WHERE user_id = 'vm2'"
sql -d scd_b1_main -c "INSERT INTO public.cmd_pending (tbl, pk, node, id) VALUES ('artists', jsonb_build_object('id', '$(art 4)'::uuid), 'main', '00000000-0000-0000-0000-0000000013c4')"
sql -d scd_b1_main -c "ALTER SUBSCRIPTION main_from_core SET (slot_name = NONE)" -c "DROP SUBSCRIPTION main_from_core"
val scd_b1_core "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_main_from_core')" > /dev/null
subscribe main core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'main_from_core' AND pid IS NOT NULL)" "main reads core from a new slot"
[ "$(val scd_b1_core "SELECT public.scd_resend_all() > 0")" = t ]
swept
expect "scd_resend_all heals the C update of the gap" "SELECT coalesce(max(name), 'none') FROM public.artists WHERE id = '$(art 2)'" "Gap 2 renamed"
expect "scd_resend_all heals the C insert of the gap" "SELECT count(*) FROM public.artists WHERE id = '$(art 5)'" 1
expect "scd_resend_all heals the A tombstone of the gap" "$(seen vt1)" none
ct="$(val scd_b1_core "SELECT extract(epoch FROM created_at)::text FROM public.disliked_tracks WHERE sc_user_id = 'ct1'")"
expect "scd_resend_all heals created_at of a key main inserted again in the gap (least of the same version)" \
  "SELECT extract(epoch FROM created_at)::text || '/' || (track_data ->> 'v') FROM public.disliked_tracks WHERE sc_user_id = 'ct1'" "$ct/2"
before="artists $(only_on main artists "id IN ('$(art 1)', '$(art 4)')"), user_likes_tracks $(only_on main user_likes_tracks "user_id IN ('vm1', 'vm2')"), diag_events $(only_on main diag_events "id = '00000000-0000-0000-0000-0000000013d1'"), drift $(drift main)"
[ "$before" = "artists 2, user_likes_tracks 2, diag_events 1, drift artists diag_events user_likes_tracks" ] \
  && echo "ok without repair main keeps what core removed in the gap: $before" \
  || { echo "FAIL the lost-slot shape before repair: $before"; exit 1; }
[ -z "$(drift star)" ] && echo "ok star never lost anything" || { echo "FAIL star drifted: $(drift star)"; exit 1; }

say "B13 verify: a dry run reports and changes nothing"
verify_cycle true "$tmp/verify.dry"
has "$tmp/verify.dry" "main artists {\"id\": \"$(art 1)\"} drop 1" "dry run reports the C delete of the gap"
has "$tmp/verify.dry" "main user_likes_tracks {\"user_id\": \"vm1\", \"sc_track_id\": \"t1\"} drop 1" "dry run reports the M purge of the gap"
has "$tmp/verify.dry" "main diag_events {\"id\": \"00000000-0000-0000-0000-0000000013d1\"} drop 1" "dry run reports the core-deleted A row of the gap"
grep -q '^star ' "$tmp/verify.dry" && { echo "FAIL star reported"; cat "$tmp/verify.dry"; failed=$((failed + 1)); } || echo "ok nothing reported for star"
swept
after_dry="artists $(only_on main artists "id IN ('$(art 1)', '$(art 4)')"), user_likes_tracks $(only_on main user_likes_tracks "user_id IN ('vm1', 'vm2')"), diag_events $(only_on main diag_events "id = '00000000-0000-0000-0000-0000000013d1'")"
[ "$after_dry" = "artists 2, user_likes_tracks 2, diag_events 1" ] && echo "ok the dry run changed nothing: $after_dry" || { echo "FAIL the dry run changed rows: $after_dry"; exit 1; }

say "B13 verify: repair; the owner decides, a newer re-like and a pending command survive"
verify_cycle false "$tmp/verify.fix"
echo "repair report: $(awk '{print $1, $2, $(NF-1)}' "$tmp/verify.fix" | sort | uniq -c | awk '{printf "%s %s %s x%s; ", $2, $3, $4, $1}')"
has "$tmp/verify.fix" "main artists {\"id\": \"$(art 1)\"} drop 1" "repair drops the C row core deleted"
has "$tmp/verify.fix" "main artists {\"id\": \"$(art 4)\"} drop 3" "the pending C key is reported up to the last attempt"
has "$tmp/verify.fix" "main user_likes_tracks {\"user_id\": \"vm2\", \"sc_track_id\": \"t1\"} drop 1" "repair sends the purge receipt of the re-liked row"
swept
expect "the C delete of the gap is applied on every node" "SELECT count(*) FROM public.artists WHERE id = '$(art 1)'" 0
expect "the M purge of the gap is applied on every node" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'vm1'" 0
expect "the core-deleted A row of the gap is gone on every node" "SELECT count(*) FROM public.diag_events WHERE id = '00000000-0000-0000-0000-0000000013d1'" 0
expect "the re-like newer than the receipt lives on every node" "SELECT coalesce(string_agg(wanted_state::text, ','), 'none') FROM public.user_likes_tracks WHERE user_id = 'vm2'" true
[ "$(only_on main artists "id = '$(art 4)'")" = 1 ] && echo "ok a C key with a pending command stays on main" || { echo "FAIL the pending C key was dropped"; failed=$((failed + 1)); }
[ "$(drift main)" = artists ] && echo "ok only the pending C key still differs" || { echo "FAIL drift after repair: $(drift main)"; failed=$((failed + 1)); }
sql -d scd_b1_main -c "DELETE FROM public.cmd_pending WHERE id = '00000000-0000-0000-0000-0000000013c4'"
verify_cycle false "$tmp/verify.fix2"
swept
for n in main star; do
  d="$(drift "$n")"
  [ -z "$d" ] && echo "ok after the command settles core and $n agree on every verified table: 0 diffs" || { echo "FAIL core and $n still differ: $d"; failed=$((failed + 1)); }
done
agree artists "id::text LIKE '00000000-0000-0000-0000-0000000013a%'" "the gap artists after repair"
agree user_likes_tracks true "whole table user_likes_tracks after repair"
agree diag_events true "whole table diag_events after repair"

say "B13 verify: a new A row of main still in flight to core when main answers is not dropped"
pause core core_from_main
sql -d scd_b1_main -c "INSERT INTO public.diag_events (id, app_version, body, fingerprint, install, kind, os, title) VALUES ('00000000-0000-0000-0000-0000000013d2', '1', '{}', 'fp', 'in', 'k', 'os', 'flight')"
val scd_b1_core "SELECT state || ' ' || probes FROM public.scd_verify_probe(100000, 500)" > /dev/null
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'repl_checksums')" "main answers the probe with its new row in the chunk"
val scd_b1_main "SELECT sent FROM public.scd_sweep()" > /dev/null
resume core core_from_main
verify_cycle false "$tmp/verify.flight"
grep -q "13d2" "$tmp/verify.flight" && { echo "FAIL the in-flight row was acted on"; cat "$tmp/verify.flight"; failed=$((failed + 1)); } || echo "ok the answer comes after the row, core has it and does nothing"
expect "the in-flight row lives on every node" "SELECT count(*) FROM public.diag_events WHERE id = '00000000-0000-0000-0000-0000000013d2'" 1

say "B13 verify: a row core writes after its probe is not repaired against the api's older chunk"
pause main main_from_core
val scd_b1_core "SELECT probes FROM public.scd_verify_probe(100000, 500)" > /dev/null
sql -d scd_b1_core -c "INSERT INTO public.diag_events (id, app_version, body, fingerprint, install, kind, os, title) VALUES ('00000000-0000-0000-0000-0000000013d3', '1', '{}', 'fp', 'in', 'k', 'os', 'after the probe')"
sql -d scd_b1_main -c "INSERT INTO public.diag_events (id, app_version, body, fingerprint, install, kind, os, title) VALUES ('00000000-0000-0000-0000-0000000013d4', '1', '{}', 'fp', 'in', 'k', 'os', 'local')"
resume main main_from_core
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'repl_checksums')" "main answers: its chunk holds its own new row, core's probe does not"
verify_cycle false "$tmp/verify.late"
grep -q diag_events "$tmp/verify.late.recheck" && echo "ok core compares the differing chunk key by key at a new point of its stream" \
  || { echo "FAIL no key-by-key compare of diag_events"; cat "$tmp/verify.late.recheck"; failed=$((failed + 1)); }
[ -s "$tmp/verify.late" ] && { echo "FAIL rows written after the probe were repaired"; cat "$tmp/verify.late"; failed=$((failed + 1)); } || echo "ok nothing repaired: core's newer row and main's in-flight row are equal at the second compare"
expect "both rows live on every node" "SELECT count(*) FROM public.diag_events WHERE id IN ('00000000-0000-0000-0000-0000000013d3', '00000000-0000-0000-0000-0000000013d4')" 2

say "B13 verify: a probe that meets a pending resend of its table waits for the sweeper"
sql -d scd_b1_main -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000013b2', 'Waits', 'waits', 'sc')"
sql -d scd_b1_main -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('artists', jsonb_build_object('id', '00000000-0000-0000-0000-0000000013bf'::uuid))"
val scd_b1_core "SELECT probes FROM public.scd_verify_probe(100000, 500)" > /dev/null
reach
waiting() { val scd_b1_main "SELECT count(*) FILTER (WHERE range LIKE 'wait %') || ' waiting, ' || count(*) FILTER (WHERE range NOT LIKE 'wait %') || ' answered' FROM public.repl_checksums WHERE node = 'main' AND tbl = 'artists'"; }
got="$(waiting)"
[ "$got" = "1 waiting, 0 answered" ] && echo "ok main holds the artists probe while a resend of artists is pending ($got)" || { echo "FAIL held probe: $got"; failed=$((failed + 1)); }
got="$(val scd_b1_main "SELECT state || ' gone=' || gone FROM public.scd_sweep()") then $(waiting)"
[ "$got" = "ran gone=1 then 0 waiting, 1 answered" ] && echo "ok the sweep ships the resend, then answers the held probe ($got)" || { echo "FAIL held probe after the sweep: $got"; failed=$((failed + 1)); }
calm
[ "$(val scd_b1_core "SELECT count(*) FROM public.repl_checksums WHERE node = 'main' AND tbl = 'artists'")" = 1 ] && echo "ok the late answer reaches core" || { echo "FAIL the late answer did not reach core"; failed=$((failed + 1)); }
verify_cycle false "$tmp/verify.wait"
has "$tmp/verify.wait" 'main artists {"id": "00000000-0000-0000-0000-0000000013b2"} drop 1' "the late answer drops main's stray C row"
swept
[ -z "$(drift main)" ] && echo "ok core and main agree again" || { echo "FAIL main drift: $(drift main)"; failed=$((failed + 1)); }

say "B13 verify: an api node still on the 0202 applier ignores verify ops (rollout order)"
newdef="$(val scd_b1_star "SELECT pg_get_functiondef('public.scd_on_repl_repair()'::regprocedure)")"
sql -d scd_b1_star -c "CREATE OR REPLACE FUNCTION public.scd_on_repl_repair()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS \$f\$
DECLARE
    cls text := public.scd_table_class(NEW.tbl);
BEGIN
    IF NEW.src_node <> public.scd_node()
        AND ((public.scd_role() = 'core' AND cls IN ('M', 'A', 'K')) OR (public.scd_role() = 'api' AND cls IN ('C', 'M', 'A'))) THEN
        PERFORM public.scd_apply_row(NEW.tbl, NEW.\"row\");
    END IF;
    RETURN NULL;
END
\$f\$"
sql -d scd_b1_star -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000013b1', 'Old', 'old', 'sc')"
errors_before="$(val postgres "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats")"
val scd_b1_core "SELECT probes FROM public.scd_verify_probe(100000, 500)" > /dev/null
calm
got="answers $(val scd_b1_star "SELECT count(*) FROM public.repl_checksums")/$(val scd_b1_star "SELECT count(*) FROM public.repl_resend"), apply errors $(( $(val postgres "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats") - errors_before ))"
[ "$got" = "answers 0/0, apply errors 0" ] && echo "ok the old applier skips every op and its apply worker keeps running ($got)" || { echo "FAIL old applier: $got"; failed=$((failed + 1)); }
sql -d scd_b1_star -c "$newdef"
verify_cycle false "$tmp/verify.old"
has "$tmp/verify.old" 'star artists {"id": "00000000-0000-0000-0000-0000000013b1"} drop 1' "after the upgrade the next cycle drops star's stray C row"
swept
[ -z "$(drift star)" ] && echo "ok core and star agree again" || { echo "FAIL star drift: $(drift star)"; failed=$((failed + 1)); }

[ "$failed" = 0 ] || { echo "FAIL $failed B13 checks"; exit 1; }
