ld_from=0285
ld_to=0292
ld_job=00000000-0000-0000-0000-0000000000d1
ld_lease=00000000-0000-0000-0000-0000000000d2
ld_lookup="$root/jobs/queries/lyrics"
ld_cols="'attempts', 'last_attempt_at', 'claim_job_id', 'claim_job_generation', 'claim_job_lease_id', 'claim_state_generation', 'claim_expires_at'"
ld_mk=0

ld_apply() {
  local f b v
  for f in "$root"/api/migrations/*.sql; do
    b="${f##*/}"
    v="${b%%_*}"
    [[ "$v" < "$2" || "$v" > "$3" ]] && continue
    if [ "$(head -1 "$f")" = "-- no-transaction" ]; then
      sql -d "scd_b1_$1" -c "SET client_min_messages = error" -f - < "$f" > /dev/null
    else
      sql -1 -d "scd_b1_$1" -c "SET client_min_messages = error" -f - < "$f" > /dev/null
    fi
  done
}

ld_seed() {
  sql -d scd_b1_ld_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
    SELECT gen_random_uuid(), 'urn:t:ld' || n, 'ld' || n, 'Ld ' || n, 'ld ' || n, 1000 FROM generate_series($1, $2) AS n" > /dev/null
}

ld_reach() {
  local c
  ld_mk=$((ld_mk + 1))
  c="$(printf 'd%03x' "$ld_mk")"
  mark ld_core "$c"
  wait_for scd_b1_ld_main "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$c')" "$1: main applied core's stream"
  wait_for scd_b1_ld_star "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$c')" "$1: star applied core's stream"
}

ld_settle() {
  local claimed track generation
  claimed="$(sql -At -d scd_b1_ld_core -c "PREPARE claim AS $(cat "$ld_lookup/lookup_claim_target.sql")" -c "EXECUTE claim('$ld_job', 1, '$ld_lease', '$1', 900)")"
  track="$(printf '%s' "$claimed" | cut -d'|' -f1)"
  generation="$(printf '%s' "$claimed" | cut -d'|' -f3)"
  [ -n "$track" ] || { echo "FAIL $2: the claim did not take $1"; exit 1; }
  want "$2: core claims and settles $1" "$(sql -At -d scd_b1_ld_core -c "PREPARE miss AS $(cat "$ld_lookup/lookup_settle_miss.sql")" -c "EXECUTE miss('$ld_job', 1, '$ld_lease', '$track', $generation, 3600, 'not_found')")" "$track"
}

ld_wake() {
  val "scd_b1_ld_$1" "SELECT id FROM public.scd_wt('lyrics.wake', jsonb_build_object('sc_track_id', '$2', 'at', now()))" > /dev/null
  wait_for scd_b1_ld_core "SELECT priority = 0 FROM public.lyrics_lookup_state WHERE sc_track_id = '$2'" "$3: a lyrics.wake from $1 runs on core"
}

ld_stat() {
  only "ld_$1" "SELECT $2 FROM pg_catalog.pg_stat_user_tables WHERE relid = 'public.lyrics_lookup_state'::regclass"
}

ld_hot() {
  local i
  for i in $(seq 1 60); do
    [ "$(ld_stat "$1" "n_tup_hot_upd > $2")" = t ] && break
    ld_mk=$((ld_mk + 1))
    mark ld_core "$(printf 'd%03x' "$ld_mk")"
    sleep 0.5
  done
  want "$1 at 0292: the settle is a HOT update" "$(ld_stat "$1" "n_tup_hot_upd > $2")" t
}

ld_same() {
  local n shown
  shown="$(only ld_core "SELECT public.scd_verify_shown('lyrics_lookup_state')::text")"
  for n in core main star; do
    only "ld_$n" "SELECT (SELECT jsonb_object_agg(e.key, e.value) FROM jsonb_each(to_jsonb(t)) e WHERE e.key = ANY ('$shown'::text[]))::text
      FROM public.lyrics_lookup_state t WHERE t.sc_track_id LIKE 'ld%' ORDER BY 1" > "$tmp/ld.$n"
    only "ld_$n" "SELECT n || ' ' || h FROM public.scd_verify_chunk('lyrics_lookup_state', NULL, NULL, '$shown'::text[])" > "$tmp/ld.$n.digest"
  done
  for n in main star; do
    cmp -s "$tmp/ld.core" "$tmp/ld.$n" || { echo "FAIL $1: lyrics_lookup_state rows differ on core and $n"; diff "$tmp/ld.core" "$tmp/ld.$n" | head -6; exit 1; }
    cmp -s "$tmp/ld.core.digest" "$tmp/ld.$n.digest" || { echo "FAIL $1: verify digest core $(cat "$tmp/ld.core.digest"), $n $(cat "$tmp/ld.$n.digest")"; exit 1; }
  done
  echo "ok $1 ($(wc -l < "$tmp/ld.core" | tr -d ' ') rows over $(only ld_core "SELECT cardinality('$shown'::text[])") columns, verify digest $(cat "$tmp/ld.core.digest") on core, main and star)"
}

ld_errors() {
  want "$1: no apply error and every apply worker up" "$(val postgres "SELECT coalesce(sum(s.apply_error_count), 0) || ' ' || (SELECT count(*) FROM pg_catalog.pg_stat_subscription w WHERE w.subname LIKE 'ld\_%' AND w.relid IS NULL AND w.leader_pid IS NULL AND w.pid IS NOT NULL)
    FROM pg_catalog.pg_stat_subscription_stats s WHERE s.subname LIKE 'ld\_%'")" "0 4"
  want "$1: no dead command on core" "$(only ld_core "SELECT count(*) FROM public.cmd_dead")" 0
}

say "lyrics B3: core drops the claim columns first, then main and star, with replication running"
fresh scd_b1_ld_tpl
for f in "$root"/api/migrations/*.sql; do
  b="${f##*/}"
  [[ "${b%%_*}" < "$ld_from" ]] || break
  sql -d scd_b1_ld_tpl -c "SET client_min_messages = error" -f - < "$f" > /dev/null
done
for n in core main star; do
  fresh "scd_b1_ld_$n" scd_b1_ld_tpl
  role=api; [ "$n" = core ] && role=core
  val "scd_b1_ld_$n" "SELECT count(*) FROM public.scd_configure('$n', '$role', 'split')" > /dev/null
  val "scd_b1_ld_$n" "SELECT count(*) FROM public.scd_grant_repl()" > /dev/null
  val "scd_b1_ld_$n" "SELECT count(*) FROM public.scd_sync_publications()" > /dev/null
done
subscribe ld_core ld_main "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe ld_core ld_star "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe ld_main ld_core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
subscribe ld_star ld_core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
for n in core main star; do
  wait_for "scd_b1_ld_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up on $n at 0284"
done
sql -d scd_b1_ld_core -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, lease_id, lease_generation, leased_by, lease_expires_at)
  VALUES ('$ld_job', 'lyrics.lookup', 'core_bulk', 'ld-lab', '{}'::jsonb, '$ld_lease', 1, 'lab', now() + interval '1 hour')" > /dev/null
ld_seed 1 4
ld_reach "0284 everywhere"
ld_same "0284 everywhere: lyrics_lookup_state equal"

ld_apply ld_core "$ld_from" "$ld_to"
want "core at 0292: 25 columns, published with 25, none of the 7" "$(only ld_core "SELECT (SELECT count(*) FROM pg_catalog.pg_attribute WHERE attrelid = 'public.lyrics_lookup_state'::regclass AND attnum > 0 AND NOT attisdropped)
  || ' ' || count(*) || ' ' || count(*) FILTER (WHERE c IN ($ld_cols))
  FROM pg_catalog.pg_publication_tables t, unnest(t.attnames) c WHERE t.pubname = 'scd_core_pub' AND t.tablename = 'lyrics_lookup_state'")" "25 25 0"
for n in main star; do
  want "$n still at 0284 with 32 columns" "$(only "ld_$n" "SELECT count(*) FROM pg_catalog.pg_attribute WHERE attrelid = 'public.lyrics_lookup_state'::regclass AND attnum > 0 AND NOT attisdropped")" 32
done
ld_seed 5 6
ld_wake star ld1 "core 0292, api 0284"
hot="$(ld_stat star n_tup_hot_upd)"
upd="$(ld_stat star n_tup_upd)"
ld_settle ld2 "core 0292, api 0284"
ld_reach "core 0292, api 0284"
wait_for scd_b1_ld_star "SELECT n_tup_upd > $upd FROM pg_catalog.pg_stat_user_tables WHERE relid = 'public.lyrics_lookup_state'::regclass" "star applied the settle on its 0284 table"
want "star at 0284: the settle is not HOT (the due and wake indexes hold next_run_at and updated_at)" "$(ld_stat star n_tup_hot_upd)" "$hot"
ld_same "core 0292, api 0284: lyrics_lookup_state equal"
ld_errors "core 0292, api 0284"

ld_apply ld_main "$ld_from" "$ld_to"
ld_apply ld_star "$ld_from" 0290
sql -d scd_b1_ld_star -c "SET application_name = 'ld_holder'" -c "BEGIN" -c "SELECT count(*) FROM public.lyrics_lookup_state" \
  -c "SELECT pg_sleep(8)" -c "COMMIT" > "$tmp/ld_holder.out" 2>&1 &
ld_holder=$!
wait_for scd_b1_ld_star "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'ld_holder' AND wait_event = 'PgSleep')" "star: a reader holds lyrics_lookup_state for 8 s"
ld_start="$(date +%s)"
ld_apply ld_star 0291 0291 > "$tmp/ld_drop.out" 2>&1 &
ld_drop=$!
wait_for scd_b1_ld_star "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid <> pg_catalog.pg_backend_pid() AND datname = current_database()
  AND query LIKE '%IN ACCESS EXCLUSIVE MODE%')" "star: 0291 waits for its lock"
for i in 1 2 3 4; do
  sql -At -d scd_b1_ld_star -c "SET lock_timeout = '1s'" -c "SELECT count(*) FROM public.lyrics_lookup_state" > /dev/null \
    || { echo "FAIL star: read $i waited more than 1 s behind 0291"; exit 1; }
  sleep 0.3
done
kill -0 "$ld_drop" 2>/dev/null || { echo "FAIL star: 0291 ended before the reads were done"; cat "$tmp/ld_drop.out"; exit 1; }
echo "ok star: 4 reads during 0291's wait each got the table within 1 s"
wait "$ld_holder" || { echo "FAIL star: the reader failed"; cat "$tmp/ld_holder.out"; exit 1; }
wait "$ld_drop" || { echo "FAIL star: 0291 failed"; cat "$tmp/ld_drop.out"; exit 1; }
ld_took=$(($(date +%s) - ld_start))
[ "$ld_took" -ge 3 ] || { echo "FAIL star: 0291 took $ld_took s, it never waited for the reader"; exit 1; }
echo "ok star: 0291 retried its lock until the reader committed ($ld_took s)"
ld_apply ld_star 0292 "$ld_to"

for n in core main star; do
  want "$n at 0292: columns, key indexes only, fillfactor, generation_valid validated without attempts, no unpublished leftovers" "$(only "ld_$n" "SELECT
    (SELECT count(*) FROM pg_catalog.pg_attribute WHERE attrelid = 'public.lyrics_lookup_state'::regclass AND attnum > 0 AND NOT attisdropped)
    || ' ' || (SELECT string_agg(indexrelid::regclass::text, ',' ORDER BY indexrelid::regclass::text) FROM pg_catalog.pg_index WHERE indrelid = 'public.lyrics_lookup_state'::regclass)
    || ' ' || (SELECT array_to_string(reloptions, ',') FROM pg_catalog.pg_class WHERE oid = 'public.lyrics_lookup_state'::regclass)
    || ' ' || (SELECT string_agg(conname || ':' || convalidated || ':' || (pg_catalog.pg_get_constraintdef(oid) LIKE '%attempts%'), ',' ORDER BY conname)
               FROM pg_catalog.pg_constraint WHERE conrelid = 'public.lyrics_lookup_state'::regclass AND conname IN ('lyrics_lookup_state_generation_valid', 'lyrics_lookup_state_claim_valid'))
    || ' ' || (SELECT count(*) FROM public.scd_unpublished_columns WHERE tbl = 'lyrics_lookup_state')")" \
    "25 lyrics_lookup_state_pkey,lyrics_lookup_state_sc_track_id_key fillfactor=85 lyrics_lookup_state_generation_valid:true:false 0"
done
ld_seed 7 8
ld_reach "0292 everywhere"
hot_main="$(ld_stat main n_tup_hot_upd)"
hot_star="$(ld_stat star n_tup_hot_upd)"
ld_settle ld3 "0292 everywhere"
ld_hot main "$hot_main"
ld_hot star "$hot_star"
ld_wake main ld4 "0292 everywhere"
ld_wake core ld7 "0292 everywhere"
ld_reach "0292 everywhere"
ld_same "0292 everywhere: lyrics_lookup_state equal"
ld_errors "0292 everywhere"

for n in core main star; do
  for s in $(val postgres "SELECT s.subname FROM pg_catalog.pg_subscription s JOIN pg_catalog.pg_database d ON d.oid = s.subdbid WHERE d.datname = 'scd_b1_ld_$n'"); do
    sql -d "scd_b1_ld_$n" -c "ALTER SUBSCRIPTION $s DISABLE" -c "ALTER SUBSCRIPTION $s SET (slot_name = NONE)" -c "DROP SUBSCRIPTION $s"
  done
done
wait_for postgres "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd\_b1\_ld\_%' AND active)" "the B3 lab slots are idle"
for s in $(val postgres "SELECT slot_name FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd\_b1\_ld\_%'"); do
  val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
done
for d in scd_b1_ld_core scd_b1_ld_main scd_b1_ld_star scd_b1_ld_tpl; do
  sql -d postgres -c "DROP DATABASE $d WITH (FORCE)" > /dev/null
done
