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

race() {
  local label="$1" role="$2" node="$3" first="$4" second="$5" check="$6" want="$7" got
  sql -d scd_b1_race -c "SET application_name = 'scd_race'" -c "SET scd.mode = 'split'" -c "SET scd.role = '$role'" -c "SET scd.node = '$node'" \
    -c "BEGIN" -c "$first" -c "SELECT pg_sleep(2)" -c "COMMIT" > "$tmp/race.out" 2>&1 &
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
race "tombstone merge on core" core core "$replica; INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000f3\"}', 1602)" \
  "INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000000f3\"}', 3203)" \
  "SELECT count(*) || ' ' || max(v) FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000f3'" "1 3203"
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


[ "$failed" = 0 ] || { echo "FAIL $failed convergence or race checks"; exit 1; }
