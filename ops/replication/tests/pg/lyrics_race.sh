race_db=scd_b1_lyrics_race
race_seconds="${LYRICS_RACE_SECONDS:-20}"
race_clients="${LYRICS_RACE_CLIENTS:-12}"
race_tracks="${LYRICS_RACE_TRACKS:-300}"
race_queries="$root/jobs/queries/lyrics"

race_query() {
  local file="$race_queries/$1" i=$# text
  text="$(cat "$file")"
  shift
  while [ "$i" -gt 1 ]; do
    i=$((i - 1))
    text="$(printf '%s' "$text" | sed "s/\\\$$i/:${!i}/g")"
  done
  printf '%s' "$text"
}

race_script() {
  cat <<EOF
\\set n random(1, :tracks)
\\set pick random(1, 100)
\\set prio random(0, 5)
\\set c_generation -1
\\set w_generation -1
\\set zero 0
\\set claim_s 60
\\set batch 5
\\set expired 50
\\set no false
\\set yes true
\\set tries 7
SELECT 'race' || :n::text AS sc, id AS job, generation AS jgen, lease_id AS lease,
       'not_found' AS miss, 'error' AS failed, 'lab' AS why, 'lrclib' AS src,
       '[00:01.00] la' AS synced, 'la la la' AS plain, '' AS none
FROM public.background_jobs
WHERE dedup_key = 'race-' || :client_id::text \\gset
\\if :pick <= 30
$(race_query lookup_claim_target.sql job jgen lease sc claim_s) \\aset c_
\\if :c_generation >= 0
$(race_query lookup_settle_miss.sql job jgen lease c_track_id c_generation zero miss);
\\endif
\\elif :pick <= 42
$(race_query lookup_claim_target.sql job jgen lease sc claim_s) \\aset c_
\\if :c_generation >= 0
$(race_query lookup_settle_retry.sql job jgen lease c_track_id c_generation zero failed why zero);
\\endif
\\elif :pick <= 48
$(race_query lookup_claim_target.sql job jgen lease sc claim_s) \\aset c_
\\if :c_generation >= 0
$(race_query lookup_settle_found.sql job jgen lease c_track_id c_generation src synced plain no tries);
\\endif
\\elif :pick <= 52
$(race_query lookup_claim_target.sql job jgen lease sc claim_s) \\aset c_
\\if :c_generation >= 0
$(race_query lookup_settle_found.sql job jgen lease c_track_id c_generation src none plain yes tries);
\\endif
\\elif :pick <= 54
\\set claim_s 1
$(race_query lookup_claim_target.sql job jgen lease sc claim_s) \\aset c_
\\if :c_generation >= 0
\\sleep 1100 ms
$(race_query lookup_settle_miss.sql job jgen lease c_track_id c_generation zero miss);
\\endif
\\elif :pick <= 58
\\set claim_s 1
$(race_query lookup_claim_target.sql job jgen lease sc claim_s);
\\elif :pick <= 62
\\set claim_s 1
$(race_query lookup_claim_oldest.sql job jgen lease batch claim_s);
\\elif :pick <= 65
\\set claim_s 1
$(race_query lookup_claim_priority.sql job jgen lease batch claim_s zero prio);
\\elif :pick <= 69
$(race_query lookup_release_expired.sql expired);
\\elif :pick <= 76
BEGIN;
UPDATE public.tracks SET title = 'Race ' || :n::text || ' a' || :pick::text, index_priority = :prio WHERE sc_track_id = :sc::text;
DELETE FROM public.lyrics_cache WHERE sc_track_id = :sc::text;
UPDATE public.tracks SET title = 'Race ' || :n::text || ' b' || :pick::text WHERE sc_track_id = :sc::text;
END;
\\elif :pick <= 82
UPDATE public.tracks SET index_priority = :prio WHERE sc_track_id = :sc::text;
\\elif :pick <= 88
SELECT public.scd_cmd_lyrics_wake(jsonb_build_object('sc_track_id', :sc::text, 'at', now()));
\\elif :pick <= 92
SELECT track_id AS w_track_id, generation AS w_generation, wake_message_id AS w_message
FROM public.lyrics_lookup_state
WHERE sc_track_id = :sc::text AND wake_message_id IS NOT NULL AND wake_durable_at IS NULL \\aset
\\if :w_generation >= 0
$(race_query lookup_mark_wake_durable.sql w_track_id w_generation w_message);
\\endif
\\elif :pick <= 97
UPDATE public.lyrics_lookup_state SET next_run_at = now() WHERE sc_track_id = :sc::text AND next_run_at > now();
\\else
$(race_query lookup_due_wakes.sql expired);
\\endif
EOF
}

say "lyrics lookup race: a fresh database with $race_tracks tracks and $race_clients leased jobs"
fresh "$race_db" scd_b1_tpl
sql -d "$race_db" -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
    SELECT gen_random_uuid(), 'urn:race:' || n, 'race' || n, 'Race ' || n, 'race ' || n, 1000 FROM generate_series(1, $race_tracks) AS n" \
  -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, lease_id, lease_generation, leased_by, lease_expires_at)
    SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', 'race-' || n, '{}'::jsonb, gen_random_uuid(), 1, 'race', now() + interval '1 day'
    FROM generate_series(0, $race_clients - 1) AS n" > /dev/null
want "every track has a state row and a queue row" "$(only lyrics_race "SELECT count(*) FROM public.lyrics_lookup_state JOIN public.lyrics_lookup_queue USING (track_id)")" "$race_tracks"

say "lyrics lookup race: a wake waits for the track before it locks the state row"
sql -d "$race_db" -c "UPDATE public.lyrics_lookup_state SET status = 'not_found', next_run_at = now() - interval '1 minute' WHERE sc_track_id = 'race1'" > /dev/null
sql -d "$race_db" -c "SET application_name = 'race_settle'" -c "BEGIN" -c "SELECT 1 FROM public.tracks WHERE sc_track_id = 'race1' FOR UPDATE" \
  -c "SELECT pg_sleep(600)" -c "COMMIT" > /dev/null 2>&1 &
race_holder=$!
wait_for "$race_db" "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'race_settle' AND wait_event = 'PgSleep')" "a settle holds the track row of race1"
sql -d "$race_db" -c "SET application_name = 'race_wake'" -c "SELECT public.scd_cmd_lyrics_wake(jsonb_build_object('sc_track_id', 'race1', 'at', now()))" > "$tmp/race_wake.out" 2>&1 &
race_waker=$!
wait_for "$race_db" "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'race_wake' AND wait_event_type = 'Lock')" "the wake on race1 waits"
want "the state row of race1 is still free" "$(val "$race_db" "SELECT sc_track_id FROM public.lyrics_lookup_state WHERE sc_track_id = 'race1' FOR UPDATE NOWAIT" 2>&1)" race1
val "$race_db" "SELECT count(*) FROM (SELECT pg_catalog.pg_terminate_backend(pid) FROM pg_catalog.pg_stat_activity WHERE application_name = 'race_settle') AS x" > /dev/null
wait "$race_holder" || true
wait "$race_waker" || { echo "FAIL the wake on race1 failed: $(cat "$tmp/race_wake.out")"; exit 1; }
want "the wake on race1 reopened it after the settle" "$(only lyrics_race "SELECT status = 'pending' AND priority = 0 AND wake_message_id IS NOT NULL FROM public.lyrics_lookup_state WHERE sc_track_id = 'race1'")" t

say "lyrics lookup race: claims, settles, releases, wakes and track updates at once, ${race_seconds}s on $race_clients clients"
race_script > "$tmp/race.sql"
if ! $pgbench_cmd -n -M prepared -c "$race_clients" -j 4 -T "$race_seconds" -P 60 --failures-detailed --verbose-errors -D tracks="$race_tracks" -f - "$race_db" < "$tmp/race.sql" > "$tmp/race.log" 2>&1; then
  echo "FAIL pgbench stopped with an error"
  grep -v '^progress' "$tmp/race.log" | head -20
  exit 1
fi
grep -E '^(number of|tps|progress)' "$tmp/race.log" | sed 's/^/   /'
grep -i -m 5 'deadlock detected' -A 3 "$tmp/race.log" | sed 's/^/   /' || true
want "pgbench failed no transaction" "$(sed -n 's/^number of failed transactions: \([0-9]*\).*/\1/p' "$tmp/race.log")" 0
want "pgbench saw no deadlock" "$(sed -n 's/^number of deadlock failures: \([0-9]*\).*/\1/p' "$tmp/race.log")" 0
wait_for postgres "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname = '$race_db')" "every pgbench session is gone"
want "postgres counted no deadlock in the race database" "$(val postgres "SELECT deadlocks FROM pg_catalog.pg_stat_database WHERE datname = '$race_db'")" 0
work="$(only lyrics_race "SELECT (SELECT coalesce(sum(attempts), 0) > 1000 FROM public.lyrics_lookup_queue)
  AND (SELECT coalesce(sum(miss_streak), 0) > 0 FROM public.lyrics_lookup_state)
  AND (SELECT count(*) > 0 FROM public.lyrics_cache WHERE synced_lrc IS NOT NULL)
  AND (SELECT count(*) > 0 FROM public.lyrics_lookup_state WHERE last_outcome = 'recheck')")"
want "the race claimed, missed, found and rechecked" "$work" t
want "the queue still mirrors every state row" "$(only lyrics_race "$lq_drift")" 0
want "every claim left belongs to a race job" "$(only lyrics_race "SELECT count(*) FROM public.lyrics_lookup_queue AS queue
  WHERE queue.claim_job_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.background_jobs AS job WHERE job.id = queue.claim_job_id)")" 0
sql -d postgres -c "DROP DATABASE $race_db WITH (FORCE)" > /dev/null
