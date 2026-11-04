lq_drift="SELECT (
    SELECT count(*)
    FROM public.lyrics_lookup_state AS state
    LEFT JOIN public.lyrics_lookup_queue AS queue ON queue.track_id = state.track_id
    WHERE queue.track_id IS NULL
       OR (state.priority, state.next_run_at, state.created_at,
           CASE WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at END)
          IS DISTINCT FROM
          (queue.priority, queue.next_run_at, queue.created_at, queue.wake_at)
) + (
    SELECT count(*)
    FROM public.lyrics_lookup_queue AS queue
    WHERE NOT EXISTS (SELECT 1 FROM public.lyrics_lookup_state AS state WHERE state.track_id = queue.track_id)
)"
lq_queries="$root/jobs/queries/admin_maintenance"
lq_lookup="$root/jobs/queries/lyrics"
lq_job=00000000-0000-0000-0000-0000000000c1
lq_lease=00000000-0000-0000-0000-0000000000c2
lq_seed() {
  sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
    SELECT gen_random_uuid(), 'urn:t:' || '$1' || n, '$1' || n, 'Lq ' || n, 'lq ' || n, 1000 FROM generate_series(1, $2) AS n" > /dev/null
}
lq_state() { only "$1" "SELECT md5(coalesce(string_agg(sc_track_id || ':' || xmin::text, ',' ORDER BY sc_track_id), '')) FROM public.lyrics_lookup_state WHERE sc_track_id LIKE 'lq%'"; }
lq_row() { only "$1" "SELECT ctid::text || ':' || xmin::text FROM public.lyrics_lookup_state WHERE sc_track_id = '$2'"; }
lq_seen() { only "$1" "SELECT count(*) FROM lq_lab.seen"; }
lq_watch() {
  for n in main star; do
    sql -d "scd_b1_$n" -c "SET client_min_messages = warning" -c "CREATE SCHEMA lq_lab" -c "CREATE TABLE lq_lab.seen (sc_track_id text NOT NULL)" \
      -c "CREATE FUNCTION lq_lab.seen() RETURNS trigger LANGUAGE plpgsql AS \$f\$ BEGIN INSERT INTO lq_lab.seen VALUES (NEW.sc_track_id); RETURN NULL; END \$f\$" \
      -c "CREATE TRIGGER lq_seen AFTER UPDATE ON public.lyrics_lookup_state FOR EACH ROW WHEN (NEW.sc_track_id = '$1') EXECUTE FUNCTION lq_lab.seen()" \
      -c "ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER lq_seen" > /dev/null
  done
}
lq_unwatch() {
  for n in main star; do
    sql -d "scd_b1_$n" -c "DROP TRIGGER lq_seen ON public.lyrics_lookup_state" -c "DROP SCHEMA lq_lab CASCADE" > /dev/null 2>&1
  done
}
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
sql -d scd_b1_core -c "DELETE FROM public.lyrics_lookup_state WHERE sc_track_id = 'lq3'" > /dev/null
val scd_b1_main "SELECT id FROM public.scd_wt('lyrics.wake', jsonb_build_object('sc_track_id', 'lq1', 'at', now()))" > /dev/null
wait_for scd_b1_core "SELECT priority = 0 FROM public.lyrics_lookup_state WHERE sc_track_id = 'lq1'" "main's wake runs on core"
calm
want "core: one queue row per state row" "$(only core "SELECT count(*) FROM public.lyrics_lookup_queue q JOIN public.lyrics_lookup_state s USING (track_id) WHERE s.sc_track_id LIKE 'lq%'")" 2
want "core: the queue equals its mirror" "$(only core "$lq_drift")" 0
want "core: the wake from main reached the queue" "$(only core "SELECT q.priority = 0 AND q.wake_at IS NOT NULL FROM public.lyrics_lookup_queue q JOIN public.lyrics_lookup_state s USING (track_id) WHERE s.sc_track_id = 'lq1'")" t
lq_api_side
pairs lyrics_lookup_state "sc_track_id LIKE 'lq%'" "lyrics_lookup_state still replicates as before"

say "lyrics queue: a claim on core writes only the queue, a settle replicates one state update"
sql -d scd_b1_core -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, lease_id, lease_generation, leased_by, lease_expires_at)
  VALUES ('$lq_job', 'lyrics.lookup', 'core_bulk', 'lq-lab', '{}'::jsonb, '$lq_lease', 1, 'lab', now() + interval '1 hour')" > /dev/null
lq_watch lq2
core_row="$(lq_row core lq2)"; main_row="$(lq_row main lq2)"; star_row="$(lq_row star lq2)"
claimed="$(sql -At -d scd_b1_core -c "PREPARE claim AS $(cat "$lq_lookup/lookup_claim_target.sql")" -c "EXECUTE claim('$lq_job', 1, '$lq_lease', 'lq2', 900)")"
want "the claim takes lq2" "$(printf '%s' "$claimed" | cut -d'|' -f2)" lq2
track="$(printf '%s' "$claimed" | cut -d'|' -f1)"
generation="$(printf '%s' "$claimed" | cut -d'|' -f3)"
calm
want "core: the claim and the attempt are in the queue" "$(only core "SELECT claim_job_id = '$lq_job' AND claim_state_generation = $generation AND attempts = 1 AND last_attempt_at IS NOT NULL FROM public.lyrics_lookup_queue WHERE track_id = '$track'")" t
want "core: the claim left the state row as it was (ctid:xmin)" "$(lq_row core lq2)" "$core_row"
want "main: the claim left the state row as it was (ctid:xmin)" "$(lq_row main lq2)" "$main_row"
want "star: the claim left the state row as it was (ctid:xmin)" "$(lq_row star lq2)" "$star_row"
want "main applied no update for the claim" "$(lq_seen main)" 0
want "star applied no update for the claim" "$(lq_seen star)" 0
settled="$(sql -At -d scd_b1_core -c "PREPARE miss AS $(cat "$lq_lookup/lookup_settle_miss.sql")" -c "EXECUTE miss('$lq_job', 1, '$lq_lease', '$track', $generation, 3600, 'not_found')")"
want "the settle takes the claim" "$settled" "$track"
calm
want "main applied exactly one update for the settle" "$(lq_seen main)" 1
want "star applied exactly one update for the settle" "$(lq_seen star)" 1
want "star got the settled state" "$(only star "SELECT status || ' ' || miss_streak FROM public.lyrics_lookup_state WHERE sc_track_id = 'lq2'")" "not_found 1"
want "core: the settle released the claim and kept the attempt" "$(only core "SELECT claim_job_id IS NULL AND claim_expires_at IS NULL AND attempts = 1 FROM public.lyrics_lookup_queue WHERE track_id = '$track'")" t
want "core: the queue equals its mirror" "$(only core "$lq_drift")" 0
lq_unwatch
lq_api_side
pairs lyrics_lookup_state "sc_track_id LIKE 'lq%'" "lyrics_lookup_state equal on all nodes after the claim and the settle"

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
