lq_drift="SELECT (
    SELECT count(*)
    FROM public.lyrics_lookup_state AS state
    LEFT JOIN public.lyrics_lookup_queue AS queue ON queue.track_id = state.track_id
    WHERE queue.track_id IS NULL
       OR (state.priority, state.next_run_at, state.created_at,
           CASE WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at END,
           state.attempts, state.last_attempt_at, state.claim_job_id, state.claim_job_generation,
           state.claim_job_lease_id, state.claim_state_generation, state.claim_expires_at)
          IS DISTINCT FROM
          (queue.priority, queue.next_run_at, queue.created_at, queue.wake_at, queue.attempts,
           queue.last_attempt_at, queue.claim_job_id, queue.claim_job_generation,
           queue.claim_job_lease_id, queue.claim_state_generation, queue.claim_expires_at)
) + (
    SELECT count(*)
    FROM public.lyrics_lookup_queue AS queue
    WHERE NOT EXISTS (SELECT 1 FROM public.lyrics_lookup_state AS state WHERE state.track_id = queue.track_id)
)"
lq_queries="$root/jobs/queries/admin_maintenance"
lq_seed() {
  sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
    SELECT gen_random_uuid(), 'urn:t:' || '$1' || n, '$1' || n, 'Lq ' || n, 'lq ' || n, 1000 FROM generate_series(1, $2) AS n" > /dev/null
}
lq_state() { only "$1" "SELECT md5(coalesce(string_agg(sc_track_id || ':' || xmin::text, ',' ORDER BY sc_track_id), '')) FROM public.lyrics_lookup_state WHERE sc_track_id LIKE 'lq%'"; }
lq_api_side() {
  for n in main star; do
    want "$n has no queue rows" "$(only "$n" "SELECT count(*) FROM public.lyrics_lookup_queue")" 0
    want "$n publishes no queue" "$(only "$n" "SELECT count(*) FROM pg_catalog.pg_publication_tables WHERE tablename = 'lyrics_lookup_queue'")" 0
    want "$n counted no queue write" "$(only "$n" "SELECT coalesce(sum(n), 0) FROM public.scd_counters WHERE tbl = 'lyrics_lookup_queue'")" 0
  done
  want "core publishes no queue" "$(only core "SELECT count(*) FROM pg_catalog.pg_publication_tables WHERE tablename = 'lyrics_lookup_queue'")" 0
}

say "lyrics queue: core keeps a queue row next to every state row, the api nodes never get one"
lq_seed lq 3
sql -d scd_b1_core -c "UPDATE public.lyrics_lookup_state
  SET claim_job_id = gen_random_uuid(), claim_job_generation = 1, claim_job_lease_id = gen_random_uuid(),
      claim_state_generation = generation, claim_expires_at = now() + interval '15 minutes',
      attempts = attempts + 1, last_attempt_at = now(), updated_at = now()
  WHERE sc_track_id = 'lq2'" > /dev/null
sql -d scd_b1_core -c "DELETE FROM public.lyrics_lookup_state WHERE sc_track_id = 'lq3'" > /dev/null
val scd_b1_main "SELECT id FROM public.scd_wt('lyrics.wake', jsonb_build_object('sc_track_id', 'lq1', 'at', now()))" > /dev/null
wait_for scd_b1_core "SELECT priority = 0 FROM public.lyrics_lookup_state WHERE sc_track_id = 'lq1'" "main's wake runs on core"
calm
want "core: one queue row per state row" "$(only core "SELECT count(*) FROM public.lyrics_lookup_queue q JOIN public.lyrics_lookup_state s USING (track_id) WHERE s.sc_track_id LIKE 'lq%'")" 2
want "core: the queue equals its mirror" "$(only core "$lq_drift")" 0
want "core: the claim and the attempt are in the queue" "$(only core "SELECT q.claim_job_id = s.claim_job_id AND q.attempts = 1 FROM public.lyrics_lookup_queue q JOIN public.lyrics_lookup_state s USING (track_id) WHERE s.sc_track_id = 'lq2'")" t
want "core: the wake from main reached the queue" "$(only core "SELECT q.priority = 0 AND q.wake_at IS NOT NULL FROM public.lyrics_lookup_queue q JOIN public.lyrics_lookup_state s USING (track_id) WHERE s.sc_track_id = 'lq1'")" t
lq_api_side
pairs lyrics_lookup_state "sc_track_id LIKE 'lq%'" "lyrics_lookup_state still replicates as before"

say "lyrics queue: the admin backfill fills rows the trigger never saw, on core only, and replicates nothing"
sql -d scd_b1_core -c "ALTER TABLE public.lyrics_lookup_state DISABLE TRIGGER lyrics_lookup_queue_sync" > /dev/null
lq_seed lqb 40
sql -d scd_b1_core -c "ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER lyrics_lookup_queue_sync" > /dev/null
calm
want "core: 40 state rows have no queue row" "$(only core "$lq_drift")" 40
main_before="$(lq_state main)"
star_before="$(lq_state star)"
pages="$(only core "$(cat "$lq_queries/lyrics_queue_pages.sql")")"
filled="$(sql -At -d scd_b1_core -c "PREPARE fill(text, text) AS $(cat "$lq_queries/lyrics_queue_backfill.sql")" -c "EXECUTE fill('(0,0)', '($pages,0)')")"
want "the backfill inserts the 40 missing rows" "${filled##*|}" 40
checked="$(sql -At -d scd_b1_core -c "PREPARE check_queue(text, text) AS $(cat "$lq_queries/lyrics_queue_check.sql")" -c "EXECUTE check_queue('(0,0)', '($pages,0)')")"
want "the check finds no mismatch" "${checked##*|}" "{}"
want "core: the queue equals its mirror" "$(only core "$lq_drift")" 0
calm
want "main applied no state change" "$(lq_state main)" "$main_before"
want "star applied no state change" "$(lq_state star)" "$star_before"
lq_api_side
pairs lyrics_lookup_state "sc_track_id LIKE 'lq%'" "lyrics_lookup_state equal on all nodes after the backfill"
