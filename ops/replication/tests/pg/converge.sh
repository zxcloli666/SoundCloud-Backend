failed=0
mk=61440

calm() {
  local a b c out
  a="$(printf '%04x' $((mk + 1)))"; b="$(printf '%04x' $((mk + 2)))"; c="$(printf '%04x' $((mk + 3)))"
  mk=$((mk + 3))
  out="$(settle "$a" "$b" "$c")" || { echo "$out"; exit 1; }
}

quiet() {
  local round n x
  for round in 1 2 3 4 5 6 7 8; do
    calm
    n=0
    for x in $nodes; do n=$((n + $(val "scd_b1_$x" "SELECT public.scd_drain_resend()"))); done
    [ "$n" = 0 ] && return 0
  done
  echo "FAIL resend queues still not empty after 8 rounds"
  exit 1
}

expect() {
  local x got=""
  for x in $nodes; do got="$got $x=$(val "scd_b1_$x" "$2")"; done
  if [ "$got" = " core=$3 main=$3 star=$3" ]; then
    echo "ok $1"
  else
    echo "FAIL $1:$got (want $3 on every node)"
    failed=$((failed + 1))
  fi
}

dislike() {
  sql -d "scd_b1_$1" -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data)
    VALUES (public.scd_uuid('disliked_tracks', '$2', 'x'), '$2', 'x', '{\"v\": $3}')"
}

redislike() {
  sql -d "scd_b1_$1" -c "UPDATE public.disliked_tracks SET track_data = '{\"v\": $3}' WHERE sc_user_id = '$2'"
}

undislike() {
  sql -d "scd_b1_$1" -c "DELETE FROM public.disliked_tracks WHERE sc_user_id = '$2'"
}

seen() {
  echo "SELECT coalesce(string_agg(track_data ->> 'v', ','), 'none') FROM public.disliked_tracks WHERE sc_user_id = '$1'"
}

hold() {
  sql -d "$1" -c "SET application_name = '$2'" -c "BEGIN" -c "$3" \
    -c "DO \$\$ BEGIN FOR i IN 1..600 LOOP PERFORM pg_stat_clear_snapshot(); IF $4 THEN RETURN; END IF; PERFORM pg_sleep(0.1); END LOOP; RAISE EXCEPTION 'the awaited state never came'; END \$\$" \
    -c "COMMIT" > "$tmp/$2.out" 2>&1 &
}

race() {
  local label="$1" role="$2" node="$3" first="$4" second="$5" check="$6" want="$7" after="${8:-SELECT 1}" got
  sql -d scd_b1_race -c "SET application_name = 'scd_race'" -c "SET scd.mode = 'split'" -c "SET scd.role = '$role'" -c "SET scd.node = '$node'" \
    -c "BEGIN" -c "$first" -c "SELECT pg_sleep(2)" -c "$after" -c "COMMIT" > "$tmp/race.out" 2>&1 &
  local holder=$!
  wait_for scd_b1_race "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_race' AND wait_event = 'PgSleep')" "$label: the first insert is in flight"
  if ! sql -d scd_b1_race -c "SET scd.mode = 'split'" -c "SET scd.role = '$role'" -c "SET scd.node = '$node'" \
    -c "SET session_replication_role = replica" -c "$second" 2> "$tmp/race.err"; then
    echo "FAIL $label: $(head -1 "$tmp/race.err")"
    failed=$((failed + 1))
    wait "$holder" || true
    return
  fi
  wait "$holder" || { echo "FAIL $label: the first session failed: $(cat "$tmp/race.out")"; failed=$((failed + 1)); return; }
  got="$(val scd_b1_race "$check")"
  if [ "$got" = "$want" ]; then
    echo "ok $label"
  else
    echo "FAIL $label: got '$got', want '$want'"
    failed=$((failed + 1))
  fi
}

say "race: two applies insert the same key at once"
fresh scd_b1_race scd_b1_tpl
replica="SET LOCAL session_replication_role = replica"
race "A merge on core" core core "$replica; INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) VALUES ('00000000-0000-0000-0000-0000000000f1', 'ra', 'x', '{\"by\": \"main\"}', 1602)" \
  "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) VALUES ('00000000-0000-0000-0000-0000000000f1', 'ra', 'x', '{\"by\": \"star\"}', 3203)" \
  "SELECT count(*) || ' ' || max(track_data ->> 'by') || ' ' || max(row_v) FROM public.disliked_tracks WHERE sc_user_id = 'ra'" "1 star 3203"
race "M merge on core" core core "$replica; INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_v) VALUES ('rm', 't', 1602)" \
  "INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v) VALUES ('rm', 't', false, 3203)" \
  "SELECT count(*) || ' ' || bool_and(wanted_state) || ' ' || max(wanted_v) FROM public.user_likes_tracks WHERE user_id = 'rm'" "1 false 3203"
race "K register on core" core core "$replica; INSERT INTO public.sc_track_counters (sc_track_id, fetched_at, play_count) VALUES ('rk', '2026-01-01', 1)" \
  "INSERT INTO public.sc_track_counters (sc_track_id, fetched_at, play_count) VALUES ('rk', '2026-01-02', 2)" \
  "SELECT count(*) || ' ' || max(play_count) FROM public.sc_track_counters WHERE sc_track_id = 'rk'" "1 2"
race "insert-only skip on core" core core "$replica; INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight) VALUES ('00000000-0000-0000-0000-0000000000f2', 're', 't', 'play', 1)" \
  "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight) VALUES ('00000000-0000-0000-0000-0000000000f2', 're', 't', 'play', 1)" \
  "SELECT count(*) FROM public.user_events WHERE sc_user_id = 're'" 1
race "two tombstones for one key on core" core core "$replica; INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000f3\"}', 1602)" \
  "INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000f3\"}', 3203)" \
  "SELECT count(*) || ' ' || max(v) FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000f3'" "2 3203"
sql -d scd_b1_race -c "SET scd.mode = 'split'" -c "SET scd.role = 'api'" -c "SET scd.node = 'main'" -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, row_v) VALUES ('00000000-0000-0000-0000-0000000000fa', 'rx', 'x', public.scd_hlc(0))" \
  -c "INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000fa\"}', 1602)"
race "a tombstone apply against a local delete of the same row on an api node (lab T3h deadlock)" api main \
  "SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'rx' FOR UPDATE" \
  "INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000fa\"}', public.scd_hlc(0) + (60000000 << 4) - 13)" \
  "SELECT (SELECT count(*) FROM public.disliked_tracks WHERE sc_user_id = 'rx') || ' ' || count(*) FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000fa'" "0 3" \
  "DELETE FROM public.disliked_tracks WHERE sc_user_id = 'rx'"
race "C owner on an api node" api main "$replica; INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000f4', 'First', 'race', 'sc')" \
  "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000f4', 'Second', 'race', 'sc')" \
  "SELECT count(*) || ' ' || max(name) FROM public.artists WHERE normalized_name = 'race'" "1 Second"
race "apply against a local insert on an api node" api main "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data) VALUES ('00000000-0000-0000-0000-0000000000f5', 'rl', 'x', '{\"by\": \"local\"}')" \
  "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) VALUES ('00000000-0000-0000-0000-0000000000f5', 'rl', 'x', '{\"by\": \"core\"}', public.scd_hlc(0) + (60000000 << 4) - 1)" \
  "SELECT count(*) || ' ' || max(track_data ->> 'by') FROM public.disliked_tracks WHERE sc_user_id = 'rl'" "1 core"
sql -d scd_b1_race -c "SET scd.mode = 'split'" -c "SET scd.role = 'core'" -c "SET scd.node = 'core'" -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) VALUES ('00000000-0000-0000-0000-0000000000f6', 'rd', 'x', '{\"by\": \"old\"}', 1602)"
race "apply into a row deleted under it on core" core core "DELETE FROM public.disliked_tracks WHERE sc_user_id = 'rd'" \
  "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) VALUES ('00000000-0000-0000-0000-0000000000f6', 'rd', 'x', '{\"by\": \"star\"}', public.scd_hlc(0) + (60000000 << 4) - 1)" \
  "SELECT count(*) || ' ' || max(track_data ->> 'by') FROM public.disliked_tracks WHERE sc_user_id = 'rd'" "1 star"
sql -d scd_b1_race -c "SET scd.mode = 'split'" -c "SET scd.role = 'api'" -c "SET scd.node = 'main'" -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000f7', 'Core', 'own', 'sc'), ('00000000-0000-0000-0000-0000000000f8', 'Core', 'cmdpurge', 'sc')"
optimistic() {
  echo "SET LOCAL scd.cmd = '$2'; UPDATE public.artists SET name = 'Opt' WHERE id = '$1';
    INSERT INTO public.cmd_pending (tbl, pk, node, id) VALUES ('artists', jsonb_build_object('id', '$1'), 'main', '$2')"
}
race "a core row behind a local optimistic write waits for it and is skipped" api main \
  "$(optimistic 00000000-0000-0000-0000-0000000000f7 00000000-0000-0000-0000-0000000000c7)" \
  "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000f7', 'Repair', 'own', 'sc')" \
  "SELECT max(name) FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000f7'" Opt
race "a cmd_purge behind a local optimistic write waits for it and keeps the row" api main \
  "$(optimistic 00000000-0000-0000-0000-0000000000f8 00000000-0000-0000-0000-0000000000c8)" \
  "INSERT INTO public.cmd_purge (node, id, tbl, pk) VALUES ('main', '00000000-0000-0000-0000-0000000000c9', 'artists', '{\"id\": \"00000000-0000-0000-0000-0000000000f8\"}')" \
  "SELECT count(*) || ' ' || max(name) FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000f8'" "1 Opt"
sql -d postgres -c "DROP DATABASE scd_b1_race WITH (FORCE)"

say "A tombstones: an update newer than a delete reaches core first and lives on every node"
dislike star cv1 1
calm
pause core core_from_main
undislike main cv1
redislike star cv1 2
wait_for scd_b1_core "SELECT track_data ->> 'v' = '2' FROM public.disliked_tracks WHERE sc_user_id = 'cv1'" "core has star's update before main's delete"
resume core core_from_main
quiet
expect "the newer update survives the older key-only delete" "$(seen cv1)" 2

say "A tombstones: the delete reaches core first, the newer update is dropped there as update_missing"
dislike star cv2 1
calm
pause star star_from_core
pause core core_from_star
undislike main cv2
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'cv2')" "core applies main's delete"
redislike star cv2 2
resume core core_from_star
resume star star_from_core
quiet
expect "the newer update comes back to core and main" "$(seen cv2)" 2

say "A tombstones: core has no row, only the tombstone travels and meets a local insert in flight (lab T3h)"
dislike star cv3 1
calm
pause main main_from_core
undislike star cv3
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'cv3')" "core applies star's delete"
mk=$((mk + 1)); echo3="$(printf '%04x' "$mk")"
mark core "$echo3"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$echo3')" "star consumed the echo of its delete, so it cannot wait on the re-insert"
tomb3="SELECT 1 FROM public.repl_tombstones WHERE tbl = 'disliked_tracks' AND pk = jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'cv3', 'x')) AND v % 16 = 2"
sql -d scd_b1_star -c "SET application_name = 'scd_skew'" -c "BEGIN" \
  -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data) VALUES (public.scd_uuid('disliked_tracks', 'cv3', 'x'), 'cv3', 'x', '{\"v\": 3}')" \
  -c "DO \$\$ BEGIN FOR i IN 1..300 LOOP EXIT WHEN EXISTS ($tomb3); PERFORM pg_sleep(0.1); END LOOP; END \$\$" \
  -c "SELECT CASE WHEN EXISTS ($tomb3) THEN 'tombstone seen in flight' END" -c "COMMIT" > "$tmp/skew.out" 2>&1 &
skew=$!
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_skew' AND wait_event = 'PgSleep')" "star holds an uncommitted re-insert"
undislike main cv3
wait "$skew" || { cat "$tmp/skew.out"; exit 1; }
grep -q 'tombstone seen in flight' "$tmp/skew.out" \
  && echo "ok main's tombstone reaches star without a delete while the insert is in flight" \
  || { echo "FAIL main's tombstone did not reach star while the insert was in flight"; cat "$tmp/skew.out"; exit 1; }
redislike star cv3 4
resume main main_from_core
quiet
expect "star's later update wins over main's delete on every node" "$(seen cv3)" 4

say "A tombstones: an older update loses to a newer delete"
dislike star cv4 1
calm
pause star star_from_core
pause core core_from_star
redislike star cv4 2
undislike main cv4
resume core core_from_star
resume star star_from_core
quiet
expect "the delete wins over the older update" "$(seen cv4)" none

say "A tombstones: a re-insert after a seen delete beats a tombstone that is ahead of the clock"
sql -d scd_b1_core -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v)
      VALUES (public.scd_uuid('disliked_tracks', 'cv5', 'x'), 'cv5', 'x', '{\"v\": 1}', public.scd_hlc(0) + (60000000 << 4))"
calm
undislike main cv5
calm
dislike star cv5 2
quiet
expect "the re-insert lives on every node" "$(seen cv5)" 2

say "A tombstones: deletes made on main while core's slot on main is lost reach core through scd_resend_all (lab T9)"
dislike main ls1 1
dislike main ls2 1
calm
pause core core_from_main
undislike main ls1
redislike main ls2 2
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_main SET (slot_name = NONE)" -c "DROP SUBSCRIPTION core_from_main"
val scd_b1_main "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_core_from_main')" > /dev/null
subscribe core main "scd_api_pub, scd_cmd_pub" "origin = none"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'core_from_main' AND pid IS NOT NULL)" "core reads main from a new slot"
calm
[ "$(val scd_b1_core "SELECT count(*) FROM public.disliked_tracks WHERE sc_user_id = 'ls1'")" = 1 ] || { echo "FAIL the lost slot did not lose main's delete"; exit 1; }
[ "$(val scd_b1_main "SELECT public.scd_resend_all() > 0")" = t ]
quiet
expect "the delete made in the gap is applied on every node" "$(seen ls1)" none
expect "the update made in the gap reaches every node" "$(seen ls2)" 2
expect "one tombstone per delete on every node, still main's version" \
  "SELECT count(*) || ' ' || bool_and(v % 16 = 2) FROM public.repl_tombstones WHERE pk = jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'ls1', 'x'))" "1 true"

say "A tombstones: star updates rows main deleted before the delete reaches star (lab T7 W3); the newer rows come back through the drain"
w3_keys="SELECT jsonb_build_object('id', public.scd_uuid('disliked_tracks', u, 'x')) FROM unnest(ARRAY['w3a', 'w3b', 'w3c']) u"
for u in w3a w3b w3c; do dislike main "$u" 1; done
calm
pause star star_from_core
for u in w3a w3b w3c; do undislike main "$u"; done
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id LIKE 'w3%')" "core applies main's deletes"
sql -d scd_b1_star -c "UPDATE public.disliked_tracks SET track_data = track_data || '{\"late\": 1}' WHERE sc_user_id LIKE 'w3%'"
resume star star_from_core
calm
w3="core $(val scd_b1_core "SELECT count(*) FROM public.disliked_tracks WHERE sc_user_id LIKE 'w3%'"), star $(val scd_b1_star "SELECT count(*) FROM public.disliked_tracks WHERE sc_user_id LIKE 'w3%'"), queued $(val scd_b1_star "SELECT count(*) FROM public.repl_resend WHERE tbl = 'disliked_tracks' AND pk IN ($w3_keys)")"
[ "$w3" = "core 0, star 3, queued 3" ] && echo "ok before the drain it is the lab shape: core and main dropped star's update, star keeps its rows and queues them ($w3)" \
  || { echo "FAIL before the drain: $w3 (want core 0, star 3, queued 3)"; failed=$((failed + 1)); }
quiet
expect "after the drain star's newer rows live on every node" \
  "SELECT count(*) FILTER (WHERE track_data ->> 'late' = '1') FROM public.disliked_tracks WHERE sc_user_id LIKE 'w3%'" 3

say "A tombstones: a local update commits while a drain holds an older image of the row; the update is not lost (lab T3h divergence)"
dislike star sx 1
calm
pause star star_from_core
pause core core_from_star
undislike main sx
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'sx')" "core applies main's delete"
redislike star sx 2
resume star star_from_core
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'disliked_tracks' AND pk = jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'sx', 'x')))" "star keeps its newer row and queues it for resend"
hold scd_b1_star scd_drain_hold "SELECT public.scd_drain_resend()" \
  "EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_late' AND wait_event_type = 'Lock')
   OR EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'sx' AND track_data ->> 'v' = '3')"
drainer=$!
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_drain_hold' AND wait_event = 'PgSleep')" "star's drain holds its image of the row"
sql -d scd_b1_star -c "SET application_name = 'scd_late'" -c "UPDATE public.disliked_tracks SET track_data = '{\"v\": 3}' WHERE sc_user_id = 'sx'" > "$tmp/late.out" 2>&1 &
late=$!
wait "$drainer" || { echo "FAIL star's drain: $(cat "$tmp/scd_drain_hold.out")"; exit 1; }
wait "$late" || { echo "FAIL star's late update: $(cat "$tmp/late.out")"; exit 1; }
resume core core_from_star
quiet
expect "the late update lives on every node, not the drain's older image" "$(seen sx)" 3

say "A tombstones: main and star resend tombstones of the same two keys in opposite orders; core applies both batches without a deadlock (lab T3h core deadlocks)"
ka="$(val scd_b1_core "SELECT public.scd_uuid('disliked_tracks', 'dla', 'x')")"
kb="$(val scd_b1_core "SELECT public.scd_uuid('disliked_tracks', 'dlb', 'x')")"
sql -d scd_b1_core -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, track_data, row_v) SELECT k::uuid, u, 'x', '{\"v\": 1}', ((extract(epoch FROM now() + interval '1 day') * 1000000)::bigint << 4) + 1 FROM (VALUES ('$ka', 'dla'), ('$kb', 'dlb')) AS r (k, u)"
calm
tomb() { sql -d "scd_b1_$1" -c "INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', jsonb_build_object('id', '$2'::uuid), 0)"; }
tomb main "$ka"; tomb main "$kb"
tomb star "$kb"; tomb star "$ka"
calm
for n in main star; do
  ord=$([ "$n" = main ] && echo 2 || echo 3)
  sql -d "scd_b1_$n" -c "DELETE FROM public.repl_resend WHERE tbl = 'disliked_tracks' AND pk ->> 'id' IN ('$ka', '$kb')" > /dev/null
  sql -d "scd_b1_$n" -c "INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_tombstones', jsonb_build_object('tbl', tbl, 'pk', pk, 'v', v)
    FROM public.repl_tombstones WHERE tbl = 'disliked_tracks' AND pk ->> 'id' IN ('$ka', '$kb') AND v % 16 = $ord ON CONFLICT DO NOTHING" > /dev/null
done
errs="SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats WHERE subname IN ('core_from_main', 'core_from_star')"
errs_before="$(val scd_b1_core "$errs")"
hold scd_b1_core scd_lock_hold "SELECT count(*) FROM (SELECT 1 FROM public.disliked_tracks WHERE id IN ('$ka', '$kb') FOR UPDATE) s" \
  "(SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE backend_type = 'logical replication apply worker' AND wait_event_type = 'Lock') >= 2"
holder=$!
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_lock_hold' AND wait_event = 'PgSleep')" "core holds both rows"
for n in main star; do val "scd_b1_$n" "SELECT public.scd_drain_resend()" > /dev/null; done
wait "$holder" || { echo "FAIL the two repair batches never both waited on core: $(cat "$tmp/scd_lock_hold.out")"; exit 1; }
echo "ok both repair batches waited on core's locked rows"
quiet
errs_after="$(val scd_b1_core "$errs")"
[ "$errs_after" = "$errs_before" ] && echo "ok no apply error on core from the two repair batches" \
  || { echo "FAIL core apply errors went from $errs_before to $errs_after (deadlock between repair batches)"; failed=$((failed + 1)); }
expect "both rows live on every node above their tombstones" "SELECT count(*) FROM public.disliked_tracks WHERE id IN ('$ka', '$kb')" 2

say "C: a kind on an api node touches artists, core's derived has_sc_account comes back as a core write and stays out of the hash"
kind_sql="CREATE OR REPLACE FUNCTION public.scd_cmd_match_sc_account(args jsonb) RETURNS jsonb LANGUAGE plpgsql SET search_path = public, pg_catalog AS \$f\$
BEGIN
    INSERT INTO public.artist_sc_accounts (artist_id, sc_user_id, role, source, verified, created_at, updated_at)
    VALUES ((args ->> 'artist_id')::uuid, args ->> 'sc_user_id', 'main', 'sc', true, (args ->> 'at')::timestamptz, (args ->> 'at')::timestamptz)
    ON CONFLICT (artist_id, sc_user_id) DO NOTHING;
    UPDATE public.artists SET sc_user_id = coalesce(sc_user_id, args ->> 'sc_user_id') WHERE id = (args ->> 'artist_id')::uuid;
    RETURN jsonb_build_object('output', true, 'touched', jsonb_build_array(
        jsonb_build_object('tbl', 'artist_sc_accounts', 'pk', jsonb_build_object('artist_id', (args ->> 'artist_id')::uuid, 'sc_user_id', args ->> 'sc_user_id')),
        jsonb_build_object('tbl', 'artists', 'pk', jsonb_build_object('id', (args ->> 'artist_id')::uuid))));
END
\$f\$"
for n in $nodes; do sql -d "scd_b1_$n" -c "$kind_sql"; done
sql -d scd_b1_core -c "INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000a9', 'Acct', 'acct', 'sc')"
calm
cid="$(val scd_b1_main "SELECT id FROM public.scd_wt('match.sc_account', jsonb_build_object('artist_id', '00000000-0000-0000-0000-0000000000a9', 'sc_user_id', 'scu-9', 'at', '2026-10-30 00:00:00+00'))")"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE id = '$cid' AND done_at IS NOT NULL)" "core ran main's command"
calm
touched="$(val scd_b1_main "SELECT touched FROM public.cmd_outbox WHERE id = '$cid'")"
equal="$(val scd_b1_core "SELECT count(*) FILTER (WHERE public.scd_row_hash(k ->> 'tbl', k -> 'pk') = k ->> 'hash') || '/' || count(*) FROM jsonb_array_elements('$touched'::jsonb) k")"
[ "$equal" = 2/2 ] && echo "ok core's rows hash like main's kind-time rows" || { echo "FAIL core's hashes differ from main's kind-time hashes: $equal equal"; failed=$((failed + 1)); }
reasserted="$(val scd_b1_core "SELECT count(*) FROM public.cmd_reassert WHERE id = '$cid'")"
[ "$reasserted" = 2 ] && echo "ok core reasserts both keys only because both tables are reassert: always" || { echo "FAIL core reasserted $reasserted row(s) for main's command"; failed=$((failed + 1)); }
[ "$(val scd_b1_core "SELECT count(*) FROM public.cmd_dead WHERE id = '$cid'")" = 0 ] || { echo "FAIL main's command died on core"; exit 1; }
expect "the artist row is equal on every node with core's derived flag" \
  "SELECT sc_user_id || ' ' || has_sc_account FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000a9'" "scu-9 true"
pairs artists "id = '00000000-0000-0000-0000-0000000000a9'" "artist row equal on all nodes"
pairs artist_sc_accounts "artist_id = '00000000-0000-0000-0000-0000000000a9'" "account row equal on all nodes"

say "C: core columns reach main and star as core writes (artists.has_sc_account above, lyrics_cache.fts from core and from an api kind)"
published="$(val scd_b1_core "SELECT string_agg(l.tbl || '.' || c.col || '=' || coalesce(c.col = ANY (l.cols), true)::text, ',' ORDER BY l.tbl)
  FROM public.scd_core_columns() c JOIN public.scd_publication_lists() l ON l.tbl = c.tbl AND l.pubname = 'scd_core_pub'")"
[ "$published" = "artists.has_sc_account=true,lyrics_cache.fts=true" ] && echo "ok core columns are published to main and star: $published" \
  || { echo "FAIL core columns not all published: $published"; exit 1; }
lyrics_sql="CREATE OR REPLACE FUNCTION public.scd_cmd_match_sc_account(args jsonb) RETURNS jsonb LANGUAGE plpgsql SET search_path = public, pg_catalog AS \$f\$
BEGIN
    INSERT INTO public.lyrics_cache (sc_track_id, plain_text, source, created_at, updated_at)
    VALUES (args ->> 'track', args ->> 'text', 'lrclib', (args ->> 'at')::timestamp, (args ->> 'at')::timestamptz)
    ON CONFLICT (sc_track_id) DO UPDATE SET plain_text = EXCLUDED.plain_text;
    RETURN jsonb_build_object('output', true, 'touched', jsonb_build_array(
        jsonb_build_object('tbl', 'lyrics_cache', 'pk', jsonb_build_object('sc_track_id', args ->> 'track'))));
END
\$f\$"
for n in $nodes; do sql -d "scd_b1_$n" -c "$lyrics_sql"; done
sql -d scd_b1_core -c "INSERT INTO public.lyrics_cache (sc_track_id, plain_text, source, created_at) VALUES ('lyr-core', 'alpha beta', 'lrclib', '2026-10-30')"
calm
expect "fts from a core write arrives on every node" \
  "SELECT coalesce((SELECT (fts @@ to_tsquery('simple', 'alpha & beta'))::text FROM public.lyrics_cache WHERE sc_track_id = 'lyr-core'), 'none')" true
for pair in "lyr-core gamma" "lyr-api zeta"; do
  track="${pair%% *}"; word="${pair##* }"
  cid="$(val scd_b1_main "SELECT id FROM public.scd_wt('match.sc_account', jsonb_build_object('track', '$track', 'text', '$word words', 'at', '2026-10-30 00:00:00'))")"
  wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE id = '$cid' AND done_at IS NOT NULL)" "core ran main's lyrics command for $track"
  [ "$(val scd_b1_core "SELECT count(*) FROM public.cmd_reassert WHERE id = '$cid'")" = 0 ] && echo "ok core reasserted nothing for $track" \
    || { echo "FAIL core reasserted main's lyrics command for $track"; failed=$((failed + 1)); }
done
quiet
expect "fts computed by core for main's kind arrives on every node, the old fts is gone" \
  "SELECT string_agg(sc_track_id || '=' || (fts @@ to_tsquery('simple', CASE sc_track_id WHEN 'lyr-core' THEN 'gamma & words' ELSE 'zeta & words' END))::text
     || '/' || (fts @@ to_tsquery('simple', 'alpha'))::text, ',' ORDER BY sc_track_id) FROM public.lyrics_cache WHERE sc_track_id LIKE 'lyr-%'" \
  "lyr-api=true/false,lyr-core=true/false"
expect "has_sc_account set by core's trigger is still core's value on every node after the drains" \
  "SELECT has_sc_account::text FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000a9'" true
pairs lyrics_cache "sc_track_id LIKE 'lyr-%'" "lyrics rows equal on all nodes, fts included"
pairs artists "id = '00000000-0000-0000-0000-0000000000a9'" "artist row equal on all nodes, has_sc_account included"

say "A delete-wins: a session updated after its delete stays deleted"
sql -d scd_b1_star -c "INSERT INTO public.sessions (id) VALUES ('00000000-0000-0000-0000-0000000000e5')"
calm
pause star star_from_core
pause core core_from_star
sql -d scd_b1_main -c "DELETE FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000e5'"
sql -d scd_b1_star -c "UPDATE public.sessions SET updated_at = now() + interval '1 minute' WHERE id = '00000000-0000-0000-0000-0000000000e5'"
resume core core_from_star
resume star star_from_core
quiet
expect "the logout wins over a later touch" "SELECT count(*) FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000e5'" 0

say "M: a re-like between the caller's check and the purge lock survives"
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('pl', 't1')"
calm
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'pl'"
calm
seen_v="$(val scd_b1_core "SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = 'pl' AND NOT wanted_state")"
[ -n "$seen_v" ] || { echo "FAIL core did not see the unlike"; exit 1; }
echo "ok the caller on core sees the unlike at version $seen_v"
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = true WHERE user_id = 'pl'"
calm
purged="$(val scd_b1_core "SELECT public.scd_purge_intent('user_likes_tracks', '{\"user_id\": \"pl\", \"sc_track_id\": \"t1\"}', $seen_v)")"
quiet
expect "the purge skips a row that changed under the caller (purged=$purged), the re-like lives on every node" \
  "SELECT coalesce(string_agg(wanted_state::text, ','), 'none') FROM public.user_likes_tracks WHERE user_id = 'pl'" true

say "M: a stale row after its purge is refused on core and on the api nodes; a re-like after the purge lives"
sr_pk='{"user_id": "sr", "sc_track_id": "t1"}'
blocked() { val "scd_b1_$1" "SELECT n FROM public.scd_counters WHERE counter = 'purge_block' AND tbl = 'user_likes_tracks'"; }
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('sr', 't1')"
calm
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'sr'"
calm
sr_v="$(val scd_b1_core "SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = 'sr' AND NOT wanted_state")"
stale="$(val scd_b1_main "SELECT public.scd_row_json('user_likes_tracks', '$sr_pk')")"
[ -n "$sr_v" ] && [ -n "$stale" ] || { echo "FAIL core did not see the unlike of sr"; exit 1; }
core_before="$(blocked core)"; main_before="$(blocked main)"; star_before="$(blocked star)"
pause star star_from_core
[ "$(val scd_b1_core "SELECT public.scd_purge_intent('user_likes_tracks', '$sr_pk', $sr_v)")" = t ] || { echo "FAIL core did not purge sr"; exit 1; }
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'sr')" "main applies the purge"
sql -d scd_b1_star -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_likes_tracks', '$sr_pk')" -c "SELECT public.scd_drain_resend()" > /dev/null
wait_for scd_b1_core "SELECT n > $core_before FROM public.scd_counters WHERE counter = 'purge_block' AND tbl = 'user_likes_tracks'" "core refuses star's stale resend and counts it"
[ "$(val scd_b1_core "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'sr'")" = 0 ] \
  && echo "ok core keeps the purged row gone" || { echo "FAIL core took back the purged row"; failed=$((failed + 1)); }
sql -d scd_b1_core -c "INSERT INTO public.repl_repair (id, src_node, tbl, pk, \"row\") VALUES ('00000000-0000-0000-0000-0000000000a5', 'core', 'user_likes_tracks', '$sr_pk', '$stale')" \
  -c "DELETE FROM public.repl_repair WHERE id = '00000000-0000-0000-0000-0000000000a5'" > /dev/null
wait_for scd_b1_main "SELECT n > $main_before FROM public.scd_counters WHERE counter = 'purge_block' AND tbl = 'user_likes_tracks'" "main refuses a stale repair from core and counts it"
resume star star_from_core
quiet
[ "$(blocked star)" -gt "$star_before" ] && echo "ok star refuses the stale repair after its own purge" \
  || { echo "FAIL star did not refuse the stale repair"; failed=$((failed + 1)); }
expect "the purged row stays gone on every node" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'sr'" 0
sql -d scd_b1_star -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('sr', 't1')"
quiet
expect "a re-like after the purge lives on every node, above the purged version" \
  "SELECT coalesce(string_agg((wanted_state AND wanted_v > $sr_v)::text, ','), 'none') FROM public.user_likes_tracks WHERE user_id = 'sr'" true

agree() {
  local n a b d out="" bad=0
  for n in $nodes; do rows "$n" "$1" "$2" > "$tmp/rows.$n"; done
  for p in core:main core:star main:star; do
    a="${p%%:*}"; b="${p##*:}"
    d="$(diff "$tmp/rows.$a" "$tmp/rows.$b" | grep -c '^[<>]' || true)"
    out="$out $a/$b=$d"
    [ "$d" = 0 ] || bad=1
  done
  if [ "$bad" = 0 ]; then
    echo "ok $3 ($1 rows $(wc -l < "$tmp/rows.core" | tr -d ' '), differing lines$out)"
  else
    echo "FAIL $3: $1 differing lines$out"
    failed=$((failed + 1))
  fi
}

agree disliked_tracks "sc_track_id <> 'other'" "whole table disliked_tracks after the tombstone cases, except the different-id R6 key"
agree sessions true "whole table sessions after the tombstone cases"
agree repl_tombstones true "whole table repl_tombstones after the tombstone cases"
agree user_likes_tracks true "whole table user_likes_tracks after the purge case"


[ "$failed" = 0 ] || { echo "FAIL $failed convergence or race checks"; exit 1; }
