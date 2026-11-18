rv_from=0451
rv_to=0452
rv_tables="'remote_grants', 'remote_link_codes'"
rv_kind=auth.cleanup_remote_links
rv_grant=00000000-0000-0000-0000-0000000000e1
rv_code=00000000-0000-0000-0000-0000000000e2
rv_gone=00000000-0000-0000-0000-0000000000e3
rv_mk=0

rv_reach() {
  local c
  rv_mk=$((rv_mk + 1))
  c="$(printf 'e%03x' "$rv_mk")"
  mark rv_core "$c"
  wait_for scd_b1_rv_main "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$c')" "$1: main applied core's stream"
  wait_for scd_b1_rv_star "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$c')" "$1: star applied core's stream"
}

rv_dislike() {
  sql -d "scd_b1_rv_$1" -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES (public.scd_uuid('disliked_tracks', 'rv', '$2'), 'rv', '$2')" > /dev/null
  for n in core main star; do
    wait_for "scd_b1_rv_$n" "SELECT EXISTS (SELECT 1 FROM public.disliked_tracks WHERE sc_user_id = 'rv' AND sc_track_id = '$2')" "$3: a dislike written on $1 is on $n"
  done
}

rv_errors() {
  want "$1: no apply error and every apply worker up" "$(val postgres "SELECT coalesce(sum(s.apply_error_count), 0) || ' ' || (SELECT count(*) FROM pg_catalog.pg_stat_subscription w WHERE w.subname LIKE 'rv\_%' AND w.relid IS NULL AND w.leader_pid IS NULL AND w.pid IS NOT NULL)
    FROM pg_catalog.pg_stat_subscription_stats s WHERE s.subname LIKE 'rv\_%'")" "0 4"
  want "$1: no dead command on core" "$(only rv_core "SELECT count(*) FROM public.cmd_dead")" 0
}

rv_left() {
  only "rv_$1" "SELECT (SELECT count(*) FROM pg_catalog.pg_class WHERE relnamespace = 'public'::regnamespace AND relname IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM pg_catalog.pg_proc WHERE pronamespace = 'public'::regnamespace AND proname ~ 'remote_(grants|link_codes)$')
    || ' ' || (SELECT count(*) FROM pg_catalog.pg_publication_tables WHERE tablename IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.scd_publication_lists() WHERE tbl IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM pg_catalog.pg_subscription_rel r WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_class c WHERE c.oid = r.srrelid))
    || ' ' || (SELECT count(*) FROM public.repl_resend WHERE tbl IN ($rv_tables) OR pk ->> 'tbl' IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.repl_checksums WHERE tbl IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.repl_tombstones WHERE tbl IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.background_job_failures WHERE kind = '$rv_kind')"
}

say "speaker link tables: core drops them first, then main and star, with replication running"
fresh scd_b1_rv_tpl
for f in "$root"/api/migrations/*.sql; do
  b="${f##*/}"
  [[ "${b%%_*}" < "$rv_from" ]] || break
  sql -d scd_b1_rv_tpl -c "SET client_min_messages = error" -f - < "$f" > /dev/null
done
for n in core main star; do
  fresh "scd_b1_rv_$n" scd_b1_rv_tpl
  role=api; [ "$n" = core ] && role=core
  val "scd_b1_rv_$n" "SELECT count(*) FROM public.scd_configure('$n', '$role', 'split')" > /dev/null
  val "scd_b1_rv_$n" "SELECT count(*) FROM public.scd_grant_repl()" > /dev/null
  val "scd_b1_rv_$n" "SELECT count(*) FROM public.scd_sync_publications()" > /dev/null
done
subscribe rv_core rv_main "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe rv_core rv_star "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe rv_main rv_core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
subscribe rv_star rv_core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
for n in core main star; do
  wait_for "scd_b1_rv_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up on $n before $rv_from"
done

sql -d scd_b1_rv_main -c "INSERT INTO public.remote_grants (id, sc_user_id, provider, access_hash) VALUES ('$rv_grant', 'rv', 'alice', 'rv-access')" \
  -c "INSERT INTO public.remote_link_codes (id, sc_user_id, provider, code_hash, expires_at) VALUES ('$rv_code', 'rv', 'alice', 'rv-code', now() + interval '1 hour'), ('$rv_gone', 'rv', 'salut', 'rv-gone', now() + interval '1 hour')" \
  -c "DELETE FROM public.remote_link_codes WHERE id = '$rv_gone'" > /dev/null
sql -d scd_b1_rv_core -c "INSERT INTO public.background_schedules (kind, lane, interval_seconds, next_run_at, priority, max_attempts) VALUES ('$rv_kind', 'core_fast', 60, now(), 0, 8)" \
  -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload) VALUES (gen_random_uuid(), '$rv_kind', 'core_fast', 'schedule', '{\"version\":\"1\",\"payload\":{}}')" \
  -c "INSERT INTO public.background_job_failures (id, generation, kind, lane, payload, priority, attempts, max_attempts, last_error, created_at) VALUES (gen_random_uuid(), 1, '$rv_kind', 'core_fast', '{}', 0, 8, 8, 'rv', now())" > /dev/null
for n in core star; do
  wait_for "scd_b1_rv_$n" "SELECT (SELECT count(*) FROM public.remote_grants) = 1 AND (SELECT array_agg(id::text) FROM public.remote_link_codes) = ARRAY['$rv_code']
    AND EXISTS (SELECT 1 FROM public.repl_tombstones WHERE tbl = 'remote_link_codes')" "$n has main's grant, code and tombstone"
done
for n in core main star; do
  want "$n before $rv_from: both tables published and subscribed" "$(only "rv_$n" "SELECT (SELECT count(*) FROM pg_catalog.pg_publication_tables WHERE tablename IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM pg_catalog.pg_subscription_rel WHERE srrelid IN ('public.remote_grants'::regclass, 'public.remote_link_codes'::regclass))")" \
    "$([ "$n" = core ] && echo '2 4' || echo '2 2')"
done
rv_reach "before $rv_from"
want "main and star hold core's failure row of the old kind" "$(only rv_main "SELECT count(*) FROM public.background_job_failures WHERE kind = '$rv_kind'") $(only rv_star "SELECT count(*) FROM public.background_job_failures WHERE kind = '$rv_kind'")" "1 1"

migrate_range rv_core "$rv_from" "$rv_to"
want "core at $rv_to: no table, function, publication, subscription or queue row of either table, no failure of the old kind" "$(rv_left core)" "0 0 0 0 0 0 0 0 0"
want "core at $rv_to: no schedule or queued job of the old kind" "$(only rv_core "SELECT (SELECT count(*) FROM public.background_schedules WHERE kind = '$rv_kind') + (SELECT count(*) FROM public.background_jobs WHERE kind = '$rv_kind')")" 0
want "core at $rv_to: scd_sync_publications has nothing left to change" "$(only rv_core "SELECT count(*) FROM public.scd_sync_publications()")" 0
rv_reach "core $rv_to, api before $rv_from"
for n in main star; do
  want "$n before $rv_from keeps both tables, applied core's deletes of the tombstone and the failure" "$(only "rv_$n" "SELECT (SELECT count(*) FROM pg_catalog.pg_class WHERE relnamespace = 'public'::regnamespace AND relname IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.repl_tombstones WHERE tbl IN ($rv_tables))
    || ' ' || (SELECT count(*) FROM public.background_job_failures WHERE kind = '$rv_kind')")" "2 0 0"
done
rv_dislike main rv1 "core $rv_to, api before $rv_from"
rv_dislike core rv2 "core $rv_to, api before $rv_from"
rv_errors "core $rv_to, api before $rv_from"

sql -d scd_b1_rv_main -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('remote_grants', jsonb_build_object('id', '$rv_grant'))" \
  -c "INSERT INTO public.repl_checksums (node, tbl, range, hash, at) VALUES ('main', 'remote_grants', 'wait 00000000-0000-0000-0000-0000000000e4', '{\"key\": {\"lo\": null, \"hi\": null}, \"op\": {\"hv\": 2, \"cols\": []}}', now())" > /dev/null
sql -d scd_b1_rv_star -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'remote_link_codes', 'pk', jsonb_build_object('id', '$rv_gone'), 'v', 1))" > /dev/null
migrate_range rv_main "$rv_from" "$rv_to"
migrate_range rv_star "$rv_from" "$rv_to"
rv_dislike main rv3 "$rv_to everywhere"
rv_dislike star rv4 "$rv_to everywhere"
for n in core main star; do
  want "$n at $rv_to: no table, function, publication, subscription or queue row of either table, no failure of the old kind" "$(rv_left "$n")" "0 0 0 0 0 0 0 0 0"
  want "$n at $rv_to: scd_sync_publications has nothing left to change" "$(only "rv_$n" "SELECT count(*) FROM public.scd_sync_publications()")" 0
  want "$n at $rv_to: the sweeper drains" "$(only "rv_$n" "SELECT state FROM public.scd_sweep()")" ran
done
sql -d scd_b1_rv_core -c "INSERT INTO public.repl_checksums (node, tbl, range, hash, at) VALUES ('core', '*', 'cursor', '{\"tbl\": \"remote_grants\", \"lo\": null}', now())
  ON CONFLICT (node, tbl, range) DO UPDATE SET hash = EXCLUDED.hash" > /dev/null
want "core at $rv_to: a verify cursor left on a dropped table walks on to the next table" "$(only rv_core "SELECT state || ' ' || (rel COLLATE \"C\" > 'remote_link_codes') FROM public.scd_verify_probe(1, 500)")" "ran true"
rv_reach "$rv_to everywhere"
rv_errors "$rv_to everywhere"

for n in core main star; do
  for s in $(val postgres "SELECT s.subname FROM pg_catalog.pg_subscription s JOIN pg_catalog.pg_database d ON d.oid = s.subdbid WHERE d.datname = 'scd_b1_rv_$n'"); do
    sql -d "scd_b1_rv_$n" -c "ALTER SUBSCRIPTION $s DISABLE" -c "ALTER SUBSCRIPTION $s SET (slot_name = NONE)" -c "DROP SUBSCRIPTION $s"
  done
done
wait_for postgres "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd\_b1\_rv\_%' AND active)" "the speaker drop lab slots are idle"
for s in $(val postgres "SELECT slot_name FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd\_b1\_rv\_%'"); do
  val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
done
for d in scd_b1_rv_core scd_b1_rv_main scd_b1_rv_star scd_b1_rv_tpl; do
  sql -d postgres -c "DROP DATABASE $d WITH (FORCE)" > /dev/null
done
