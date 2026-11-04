rc_pk() { echo "{\"user_id\": \"$1\", \"sc_track_id\": \"t1\"}"; }
rc_receipts() { only "$1" "SELECT coalesce(string_agg(purged_v::text, ',' ORDER BY at), 'none') FROM public.repl_purges WHERE tbl = 'user_likes_tracks' AND pk = '$2'"; }
rc_likes() { only "$1" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = '$2'"; }
rc_blocks() { only "$1" "SELECT n FROM public.scd_counters WHERE counter = 'purge_block' AND tbl = 'user_likes_tracks'"; }
rc_node_batch() { only "$1" "SELECT deleted || '|' || coalesce(held, '') FROM public.scd_node_retention_batch('repl_purges'${2:+, keep => interval '$2'})"; }
rc_old() { only "$1" "SELECT count(*) FROM public.repl_purges WHERE at < now() - interval '7 days'"; }
rc_errors() { val postgres "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats"; }
rc_acts() { awk -v u="\"$2\"" 'index($0, u) {printf "%s %s;", $(NF-1), $NF}' "$1"; }
rc_mirror() {
  sql -d scd_b1_core -c "SET scd.writer = 'mirror'" -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id, progress, synced_at, created_at, wanted_state, wanted_v, synced_for_v)
    SELECT '$1', 't1', false, now(), now(), true, f.v, f.v FROM (SELECT public.scd_purge_floor('user_likes_tracks', '$(rc_pk "$1")') AS v) AS f" > /dev/null
}
rc_purge() {
  local v
  sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('$1', 't1')" > /dev/null
  calm
  sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = '$1'" > /dev/null
  calm
  v="$(only core "SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = '$1' AND NOT wanted_state")"
  [ -n "$v" ] || { echo "FAIL core did not see the unlike of $1"; exit 1; }
  only main "SELECT public.scd_row_json('user_likes_tracks', '$(rc_pk "$1")')" > "$tmp/stale.$1"
  [ "$(only core "SELECT public.scd_purge_intent('user_likes_tracks', '$(rc_pk "$1")', $v)")" = t ] || { echo "FAIL core did not purge $1"; exit 1; }
  reach
}
rc_replay() {
  sql -d scd_b1_core -c "SELECT public.scd_repair_send('user_likes_tracks', '$(rc_pk "$1")', '$(cat "$tmp/stale.$1")')" > /dev/null
  reach
}

say "F1 receipts: a receipt only on main refuses core's row, and core's image alone cannot heal it"
rc1="$(rc_pk rc1)"
rc_main_blocks="$(rc_blocks main)"
sql -d scd_b1_main -c "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES ('user_likes_tracks', '$rc1', 0, now() - interval '3 days')" > /dev/null
rc_mirror rc1
reach
want "core mirrors rc1 at its purge floor" "$(only core "SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = 'rc1'")" 0
want "star takes rc1" "$(rc_likes star rc1)" 1
want "main refuses rc1 and counts it" "$(rc_likes main rc1) $(rc_blocks main)" "0 $((rc_main_blocks + 1))"
sql -d scd_b1_core -c "SELECT public.scd_repair_send('user_likes_tracks', '$rc1', public.scd_row_json('user_likes_tracks', '$rc1'))" > /dev/null
reach
want "core's image without the receipts, the repair before 0293, is refused again" "$(rc_likes main rc1) $(rc_blocks main)" "0 $((rc_main_blocks + 2))"

say "F1 receipts: a dry run counts the receipt difference and changes nothing"
verify_cycle true "$tmp/receipts.dry"
want "dry run reports main's extra receipt, then core's row" "$(rc_acts "$tmp/receipts.dry" rc1)" "receipts 1;row 1;"
grep -q '^star .*"rc1"' "$tmp/receipts.dry" && { echo "FAIL star reported rc1"; cat "$tmp/receipts.dry"; failed=$((failed + 1)); } || echo "ok nothing reported for star"
want "the dry run changed nothing on main" "$(rc_likes main rc1) $(rc_receipts main "$rc1")" "0 0"

say "F1 receipts: one repair cycle heals main, receipts first, then the row"
verify_cycle false "$tmp/receipts.fix"
want "the repair of rc1 is the receipts then the row, on attempt 1 only" "$(rc_acts "$tmp/receipts.fix" rc1)" "receipts 1;row 1;"
swept
want "main holds core's receipts for rc1, none" "$(rc_receipts main "$rc1")" none
expect "rc1 lives on every node" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'rc1'" 1
pairs user_likes_tracks "user_id = 'rc1'" "rc1 is the same row on every node"
want "main refused nothing more" "$(rc_blocks main)" "$((rc_main_blocks + 2))"

say "F1 receipts: a key core purged stays purged on every node after a repair"
rc_purge rc4
rc_purge rc5
rc4="$(rc_pk rc4)"
rc5="$(rc_pk rc5)"
rc4_v="$(rc_receipts core "$rc4")"
rc5_v="$(rc_receipts core "$rc5")"
expect "rc4 and rc5 are purged on every node" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id IN ('rc4', 'rc5')" 0
want "the purge receipts reached main and star" "$(rc_receipts main "$rc4") $(rc_receipts star "$rc4") $(rc_receipts main "$rc5") $(rc_receipts star "$rc5")" "$rc4_v $rc4_v $rc5_v $rc5_v"
rc_core_blocks="$(rc_blocks core)"
sql -d scd_b1_star -c "SET session_replication_role = replica" -c "SET scd.merging = 'on'" -c "SELECT public.scd_apply_row('user_likes_tracks', '$(cat "$tmp/stale.rc4")')" > /dev/null
sql -d scd_b1_main -c "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES ('user_likes_tracks', '$rc4', 0, now() - interval '3 days'), ('user_likes_tracks', '$rc5', $((rc5_v + 1000000)), now() - interval '3 days')" > /dev/null
calm
want "core refuses star's stale rc4 and counts it" "$(rc_likes core rc4) $(rc_blocks core)" "0 $((rc_core_blocks + 1))"
want "star holds the stale rc4" "$(rc_likes star rc4)" 1
rc_mirror rc5
reach
want "core mirrors rc5 above its purge" "$(only core "SELECT wanted_v > $rc5_v FROM public.user_likes_tracks WHERE user_id = 'rc5'")" t
want "star takes rc5, main's stray receipt refuses it" "$(rc_likes star rc5) $(rc_likes main rc5)" "1 0"
verify_cycle false "$tmp/receipts.purged"
want "star's stale rc4 is dropped with core's receipt" "$(rc_acts "$tmp/receipts.purged" rc4)" "drop 1;"
want "main gets core's receipts for rc5, then the row" "$(rc_acts "$tmp/receipts.purged" rc5)" "receipts 1;row 1;"
swept
expect "rc4 stays purged on every node" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'rc4'" 0
expect "rc5 lives on every node above its purge" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id = 'rc5' AND wanted_v > $rc5_v" 1
pairs user_likes_tracks "user_id IN ('rc4', 'rc5')" "rc4 and rc5 agree on every node"
want "core's purge receipts stay on every node" "$(rc_receipts core "$rc4") $(rc_receipts star "$rc4") $(rc_receipts core "$rc5") $(rc_receipts main "$rc5") $(rc_receipts star "$rc5")" "$rc4_v $rc4_v $rc5_v $rc5_v $rc5_v"
want "main's stray receipt of a key without a row is left alone" "$(rc_receipts main "$rc4")" "0,$rc4_v"
rc_main_blocks="$(rc_blocks main)"
rc_star_blocks="$(rc_blocks star)"
rc_replay rc4
rc_replay rc5
want "the stale images from before the purges are refused on main" "$(rc_blocks main)" "$((rc_main_blocks + 2))"
want "and on star" "$(rc_blocks star)" "$((rc_star_blocks + 2))"
expect "nothing came back" "SELECT count(*) FROM public.user_likes_tracks WHERE user_id IN ('rc4', 'rc5') AND wanted_v <= $rc5_v" 0

say "F2 receipts: api nodes expire receipts after 7 days like core, held while the core stream is down"
sql -d scd_b1_core -c "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES ('user_likes_tracks', '$(rc_pk rc6)', 0, now() - interval '9 days')" > /dev/null
sql -d scd_b1_main -c "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES
  ('user_likes_tracks', '$(rc_pk rc7)', 0, now() - interval '8 days'),
  ('user_likes_tracks', '$(rc_pk rc8)', 0, now() - interval '7 days 1 minute'),
  ('user_likes_tracks', '$(rc_pk rc9)', 0, now() - interval '6 days 23 hours')" > /dev/null
reach
want "core's old receipt reached main and star" "$(rc_receipts main "$(rc_pk rc6)") $(rc_receipts star "$(rc_pk rc6)")" "0 0"
want "node retention of receipts does nothing on core" "$(rc_node_batch core)" "0|"
want "core keeps its old receipt" "$(rc_receipts core "$(rc_pk rc6)")" 0
rc_main_old="$(rc_old main)"
[ "$rc_main_old" -ge 3 ] || { echo "FAIL main has $rc_main_old receipts older than 7 days, want at least 3"; exit 1; }
pause main main_from_core
want "main holds while it does not read core" "$(rc_node_batch main)" "0|main_from_core: subscription disabled"
want "main deleted nothing" "$(rc_old main)" "$rc_main_old"
resume main main_from_core
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'main_from_core' AND pid IS NOT NULL)" "main reads core again"
calm
want "a keep under 7 days is raised to 7: main drops exactly its receipts older than 7 days" "$(rc_node_batch main '1 day')" "$rc_main_old|"
want "main keeps the 6 days 23 hours receipt" "$(rc_receipts main "$(rc_pk rc9)") $(rc_old main)" "0 0"
rc_star_old="$(rc_old star)"
want "star drops its old copy of core's receipt" "$(rc_node_batch star)" "$rc_star_old|"
want "core still keeps its old receipt" "$(rc_receipts core "$(rc_pk rc6)")" 0
rc_errors_before="$(rc_errors)"
rc_core_old="$(rc_old core)"
quiet
report_both 0 0
want "core expires its own old receipts" "$(only core "SELECT deleted FROM public.scd_retention_batch('repl_purges')")" "$rc_core_old"
reach
want "core's delete of a receipt the api nodes already dropped is no apply error" "$(rc_errors)" "$rc_errors_before"
want "no receipt older than 7 days is left" "$(rc_old core) $(rc_old main) $(rc_old star)" "0 0 0"

[ "$failed" = 0 ] || { echo "FAIL $failed receipt checks"; exit 1; }
