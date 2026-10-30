only() { val "scd_b1_$1" "$2"; }
want() { [ "$2" = "$3" ] && echo "ok $1 ($2)" || { echo "FAIL $1: got '$2', want '$3'"; exit 1; }; }

batch() { only core "SELECT deleted || '|' || coalesce(held, '') FROM public.scd_retention_batch('$1', batch => ${2:-1000}, stale => interval '${3:-3 minutes}')"; }
held() { batch "$@" | cut -d'|' -f2; }
core_rows() { only core "SELECT count(*) FROM public.$1 WHERE $2"; }

report() {
  sql -d "scd_b1_$1" -c "UPDATE public.repl_checksums SET at = now() - interval '2 minutes' WHERE node = '$1' AND tbl = '*' AND range LIKE 'resend %'" > /dev/null
  only "$1" "SELECT public.scd_resend_report()" > /dev/null
}

reported() {
  wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.repl_checksums WHERE node = '$1' AND tbl = '*' AND range LIKE 'resend %' AND hash = '$2' AND at > now() - interval '20 seconds')" "core sees $1 report $2 waiting"
}

report_both() {
  report main; report star
  reported main "${1:-0}"; reported star "${2:-0}"
}

seed() {
  sql -d scd_b1_core > /dev/null <<'SQL'
INSERT INTO public.cmd_done (node, id, status, at)
SELECT 'ret', gen_random_uuid(), 'ok', now() - age FROM unnest(ARRAY[interval '8 days', interval '9 days', interval '30 days', interval '7 days 1 minute', interval '6 days 23 hours', interval '3 days', interval '1 hour']) age;
INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message, at)
SELECT 'ret', gen_random_uuid(), 'track.edit', '{}', '23514', 'seed', now() - age FROM unnest(ARRAY[interval '8 days', interval '9 days', interval '7 days 1 minute', interval '6 days 23 hours', interval '1 day']) age;
INSERT INTO public.repl_purges (tbl, pk, purged_v, at)
SELECT 'user_likes_tracks', jsonb_build_object('user_id', 'ret', 'sc_track_id', gen_random_uuid()), 1, now() - age FROM unnest(ARRAY[interval '8 days', interval '9 days', interval '7 days 1 minute', interval '6 days 23 hours', interval '2 days']) age;
INSERT INTO public.repl_tombstones (tbl, pk, at)
SELECT 'disliked_tracks', jsonb_build_object('id', gen_random_uuid(), 'ret', true), now() - age FROM unnest(ARRAY[interval '8 days', interval '9 days', interval '7 days 1 minute', interval '6 days 23 hours', interval '2 days']) age;
SQL
}

old_rows() {
  only core "SELECT (SELECT count(*) FROM public.cmd_done WHERE node = 'ret' AND at < now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.cmd_dead WHERE node = 'ret' AND at < now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.repl_purges WHERE pk ->> 'user_id' = 'ret' AND at < now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.repl_tombstones WHERE pk ->> 'ret' = 'true' AND at < now() - interval '7 days')"
}

young_rows() {
  only "$1" "SELECT (SELECT count(*) FROM public.cmd_done WHERE node = 'ret' AND at >= now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.cmd_dead WHERE node = 'ret' AND at >= now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.repl_purges WHERE pk ->> 'user_id' = 'ret' AND at >= now() - interval '7 days')
    || '/' || (SELECT count(*) FROM public.repl_tombstones WHERE pk ->> 'ret' = 'true' AND at >= now() - interval '7 days')"
}

all_old() { for t in cmd_done cmd_dead repl_purges repl_tombstones; do batch "$t" 1000 | cut -d'|' -f1; done | tr '\n' ' ' | sed 's/ $//'; }
held_all() { for t in cmd_done cmd_dead repl_purges repl_tombstones; do printf '%s ' "$(held "$t" 1000 "${1:-3 minutes}" | cut -c1-200)"; done; }

say "B7 retention: a healthy cluster, resend queues empty, every node reports"
quiet
report_both 0 0
want "the guard is clear" "$(only core "SELECT coalesce(string_agg(peer || ': ' || reason, '; '), 'clear') FROM public.scd_retention_guard()")" clear
want "an api node refuses to run retention" "$(only main "SELECT 1 FROM public.scd_retention_batch('cmd_done')" 2>&1 | grep -c 'retention runs on core only')" 1

say "B7 retention: deletes exactly the rows older than 7 days, in small batches"
seed
want "seeded old rows (cmd_done/cmd_dead/repl_purges/repl_tombstones)" "$(old_rows)" 4/3/3/3
want "seeded young rows" "$(young_rows core)" 3/2/2/2
reach
want "the young rows reached main" "$(young_rows main | cut -d/ -f3,4)" 2/2
want "a batch of 2 over 3 old tombstones deletes 2" "$(batch repl_tombstones 2)" "2|"
want "then 1" "$(batch repl_tombstones 2)" "1|"
want "then 0" "$(batch repl_tombstones 2)" "0|"
want "a keep shorter than 7 days is raised to 7 days" "$(only core "SELECT deleted FROM public.scd_retention_batch('cmd_done', keep => interval '1 hour')")" 4
want "the other tables" "$(for t in cmd_dead repl_purges; do batch "$t" 1000 | cut -d'|' -f1; done | tr '\n' ' ')" "3 3 "
want "no old row is left" "$(old_rows)" 0/0/0/0
want "the 7-days-and-1-minute rows went, the 6 days 23 hours rows stayed" "$(only core "SELECT count(*) FROM public.cmd_done WHERE node = 'ret' AND at < now() - interval '6 days 22 hours'")" 1
want "young rows untouched on core" "$(young_rows core)" 3/2/2/2
reach
wait_for scd_b1_main "SELECT count(*) = 0 FROM public.repl_tombstones WHERE pk ->> 'ret' = 'true' AND at < now() - interval '7 days'" "the deleted tombstones are gone on main too"
wait_for scd_b1_star "SELECT count(*) = 0 FROM public.repl_purges WHERE pk ->> 'user_id' = 'ret' AND at < now() - interval '7 days'" "the deleted receipts are gone on star too"
want "young rows untouched on main" "$(young_rows main | cut -d/ -f3,4)" 2/2
want "young rows untouched on star" "$(young_rows star | cut -d/ -f3,4)" 2/2

say "B7 retention: a locked table makes the batch give up after lock_timeout instead of waiting"
seed
sql -d scd_b1_core -c "BEGIN" -c "LOCK TABLE public.cmd_done IN ACCESS EXCLUSIVE MODE" -c "SELECT pg_sleep(8)" > /dev/null &
locker=$!
sleep 1
started=$(date +%s)
out="$(only core "SELECT deleted FROM public.scd_retention_batch('cmd_done')" 2>&1 || true)"
took=$(( $(date +%s) - started ))
wait $locker || true
case "$out" in *"lock timeout"*) echo "ok the batch failed with a lock timeout after ${took}s" ;; *) echo "FAIL locked batch: $out"; exit 1 ;; esac
[ "$took" -le 4 ] || { echo "FAIL the batch waited ${took}s"; exit 1; }
want "nothing was deleted by the failed batch" "$(core_rows cmd_done "node = 'ret' AND at < now() - interval '7 days'")" 4
all_old > /dev/null
want "the old rows are gone after the next run" "$(old_rows)" 0/0/0/0

say "B7 retention guard: an inactive slot (star's subscription stopped) holds every replicated table, cmd_dead still goes"
seed
pause star star_from_core
want "old rows before the run" "$(old_rows)" 4/3/3/3
want "held run deletes nothing in the three replicated tables" "$(held_all | grep -o 'scd_b1_star_from_core: slot inactive' | wc -l | tr -d ' ')" 3
want "old rows after the held run (cmd_dead is core-private and not guarded)" "$(old_rows)" 4/0/3/3
want "the guard names the slot" "$(only core "SELECT peer || ': ' || reason FROM public.scd_retention_guard() WHERE peer LIKE '%star_from_core'")" "scd_b1_star_from_core: slot inactive"
resume star star_from_core
wait_for scd_b1_core "SELECT bool_and(active) FROM pg_catalog.pg_replication_slots WHERE database = 'scd_b1_core'" "both slots are active again"
quiet
report_both 0 0
want "the guard is clear again" "$(only core "SELECT coalesce(string_agg(peer || ': ' || reason, '; '), 'clear') FROM public.scd_retention_guard()")" clear
want "the run after recovery deletes the held rows" "$(all_old | cut -d' ' -f1,3,4)" "4 3 3"
want "no old row is left" "$(old_rows)" 0/0/0/0

say "B7 retention guard: a stopped api->core subscription holds retention"
seed
pause core core_from_star
want "the guard names the subscription" "$(only core "SELECT peer || ': ' || reason FROM public.scd_retention_guard() WHERE peer = 'core_from_star'")" "core_from_star: subscription disabled"
want "nothing deleted in the replicated tables" "$(batch cmd_done | cut -d'|' -f1)/$(batch repl_purges | cut -d'|' -f1)/$(batch repl_tombstones | cut -d'|' -f1)" 0/0/0
resume core core_from_star
quiet
report_both 0 0
want "deleted after the subscription is back" "$(all_old | cut -d' ' -f1,3,4)" "4 3 3"
only core "DELETE FROM public.cmd_dead WHERE node = 'ret'" > /dev/null

say "B7 retention guard: pending resends on an api node hold retention until they are shipped"
seed
sql -d scd_b1_main -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000b7001\"}'), ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000b7002\"}')"
report main
reported main 2
want "the guard names the node and the key count" "$(only core "SELECT peer || ': ' || reason FROM public.scd_retention_guard() WHERE peer = 'main'")" "main: 2 keys wait for resend"
want "nothing deleted while main has pending resends" "$(batch cmd_done | cut -d'|' -f1)/$(batch repl_purges | cut -d'|' -f1)/$(batch repl_tombstones | cut -d'|' -f1)" 0/0/0
want "old rows are still there" "$(old_rows)" 4/3/3/3
only main "SELECT state || ' ' || gone FROM public.scd_sweep()" > /dev/null
want "main's queue is empty after its sweep" "$(only main "SELECT count(*) FROM public.repl_resend")" 0
reported main 0
want "the run after the drain deletes the old rows" "$(all_old | cut -d' ' -f1,3,4)" "4 3 3"
only core "DELETE FROM public.cmd_dead WHERE node = 'ret'" > /dev/null

say "B7 retention guard: a stale or missing report, and core's own queue, hold retention"
seed
sleep 2
want "a report older than the stale limit holds" "$(held cmd_done 1000 1s | grep -o 'resend report is' | sort -u)" "resend report is"
sql -d scd_b1_core -c "DELETE FROM public.repl_checksums WHERE node = 'star' AND tbl = '*' AND range LIKE 'resend %'" > /dev/null
want "a peer that never reported holds" "$(held cmd_done | grep -o 'star: no resend report')" "star: no resend report"
sql -d scd_b1_core -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000b7003\"}')" > /dev/null
want "core's own pending resend holds" "$(only core "SELECT count(*) FROM public.scd_retention_guard() WHERE peer = 'core' AND reason = 'keys wait for resend'")" 1
want "a lag over the limit holds (limit forced below zero)" "$(only core "SELECT count(*) > 0 FROM public.scd_retention_guard(lag_bytes => -1) WHERE reason LIKE 'slot is % bytes behind'")" t
only core "SELECT public.scd_drain_resend()" > /dev/null
report star
reported star 0
want "old rows survived all of it" "$(old_rows)" 4/3/3/3
report_both 0 0
want "and go once every report is fresh and empty" "$(all_old | cut -d' ' -f1,3,4)" "4 3 3"

say "B7 retention guard: core loses an api node's report rows, star reports again, retention resumes only on a fresh count of 0"
seed
report_both 0 0
sql -d scd_b1_core -c "DELETE FROM public.repl_checksums WHERE node = 'star' AND tbl = '*' AND range LIKE 'resend %'" > /dev/null
want "star's reports are gone on core" "$(only core "SELECT count(*) FROM public.repl_checksums WHERE node = 'star' AND tbl = '*'")" 0
want "retention pauses for the tombstone and receipt part" "$(batch repl_tombstones | cut -d'|' -f1)/$(batch repl_purges | cut -d'|' -f1)/$(batch cmd_done | cut -d'|' -f1)" 0/0/0
want "the guard says why" "$(only core "SELECT reason FROM public.scd_retention_guard() WHERE peer = 'star'")" "no resend report"
only star "SELECT public.scd_resend_report()" > /dev/null
report star
reported star 0
want "core sees star again and the guard is clear" "$(only core "SELECT coalesce(string_agg(peer || ': ' || reason, '; '), 'clear') FROM public.scd_retention_guard()")" clear
sql -d scd_b1_star -c "INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', '{\"id\": \"00000000-0000-0000-0000-0000000b7004\"}')" > /dev/null
report star
reported star 1
want "a nonzero count holds again" "$(only core "SELECT reason FROM public.scd_retention_guard() WHERE peer = 'star'")" "1 keys wait for resend"
sql -d scd_b1_core -c "UPDATE public.repl_checksums SET hash = 'garbage' WHERE node = 'star' AND tbl = '*'" > /dev/null
want "an unreadable report holds" "$(only core "SELECT reason FROM public.scd_retention_guard() WHERE peer = 'star'")" "resend report is unreadable"
sql -d scd_b1_star -c "DELETE FROM public.repl_resend" > /dev/null
report star
reported star 0
want "retention resumes once the fresh count is 0" "$(all_old | cut -d' ' -f1,3,4)" "4 3 3"
only core "DELETE FROM public.cmd_dead WHERE node = 'ret'" > /dev/null
only core "DELETE FROM public.cmd_dead WHERE node = 'ret'" > /dev/null

say "B7 metrics: the exact query file reads real values on a split core"
sub_metrics() { sql -At -d scd_b1_core < "$root/jobs/queries/repl/metrics_subscriptions.sql" | sort | cut -d'|' -f"${1:-1-4}" | tr '\n' ' '; }
want "idle: both subscriptions up, no backlog, no age" "$(sub_metrics)" "core_from_main|t|0|0 core_from_star|t|0|0 "
pause core core_from_main
want "a stopped subscription reads up = false" "$(sub_metrics 1-2)" "core_from_main|f core_from_star|t "
resume core core_from_main
wait_for scd_b1_core "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up on core"

say "B7 dead letters: replay runs the kind on core again and the effect reaches the api nodes"
for n in $nodes; do
  sql -d "scd_b1_$n" -f - > /dev/null <<'SQL'
CREATE OR REPLACE FUNCTION public.scd_cmd_track_edit(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    IF args ->> 'fail_on' = public.scd_role() THEN
        RAISE EXCEPTION 'test kind refused on %', public.scd_role() USING ERRCODE = 'check_violation';
    END IF;
    UPDATE public.tracks SET title = args ->> 'title', title_normalized = lower(args ->> 'title') WHERE urn = args ->> 'urn' RETURNING id INTO t;
    IF t IS NULL AND public.scd_role() = 'core' AND args ->> 'need_row' = 'true' THEN
        RAISE EXCEPTION 'no track %', args ->> 'urn' USING ERRCODE = 'check_violation';
    END IF;
    RETURN jsonb_build_object('output', args ->> 'title',
        'touched', CASE WHEN t IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))) END);
END
$f$;
SQL
done
sql -d scd_b1_main -c "SET scd.cmd = '00000000-0000-0000-0000-0000000b7010'" \
  -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms) VALUES ('00000000-0000-0000-0000-0000000b7011', 'urn:t:b7', 'b7', 'Local', 'local', 1000)"
cid="$(val scd_b1_main "SELECT id FROM public.scd_wt('track.edit', '{\"urn\": \"urn:t:b7\", \"title\": \"Replayed\", \"need_row\": \"true\"}')")"
wait_for scd_b1_core "SELECT status = 'dead' FROM public.cmd_done WHERE id = '$cid'" "core marks the command dead"
want "the dead letter is stored" "$(only core "SELECT sqlstate || ' ' || kind FROM public.cmd_dead WHERE id = '$cid'")" "23514 track.edit"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:b7')" "main purged its optimistic track"
want "replay of the same command fails while the track is missing and keeps the letter" "$(only core "SELECT public.scd_replay_dead('main', '$cid')")" "failed 23514: no track urn:t:b7"
want "the letter is still there with the new message" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE id = '$cid' AND message = 'no track urn:t:b7'")" 1
sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms) VALUES ('00000000-0000-0000-0000-0000000b7012', 'urn:t:b7', 'b7', 'Core', 'core', 1000)"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:b7')" "core's track reaches main"
want "replay succeeds once the track exists" "$(only core "SELECT public.scd_replay_dead('main', '$cid')")" ok
want "the letter is gone" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE id = '$cid'")" 0
wait_for scd_b1_main "SELECT title = 'Replayed' FROM public.tracks WHERE urn = 'urn:t:b7'" "the replayed edit reaches main"
wait_for scd_b1_star "SELECT title = 'Replayed' FROM public.tracks WHERE urn = 'urn:t:b7'" "the replayed edit reaches star"
want "a second replay finds nothing" "$(only core "SELECT public.scd_replay_dead('main', '$cid')")" missing
settle ee31 ee32 ee33
pairs tracks "urn = 'urn:t:b7'" "replayed track equal on all nodes"
if val scd_b1_main "SELECT public.scd_replay_dead('main', '$cid')" 2>/dev/null; then
  echo "FAIL scd_replay_dead ran on an api node"; exit 1
fi
echo "ok scd_replay_dead refuses to run on an api node"

say "B7 job_status: the queue transitions of the three kinds the api reads reach the api nodes"
bj() { only core "$1" > /dev/null; }
bj "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload) VALUES ('00000000-0000-0000-0000-0000000b7020', 'catalog.refresh', 'core_bulk', 'b7:one', '{}'), ('00000000-0000-0000-0000-0000000b7021', 'crawl.artist', 'core_bulk', 'b7:other', '{}')"
want "an insert of a read kind writes the status, another kind does not" "$(only core "SELECT string_agg(kind || ' ' || state, ',') FROM public.job_status WHERE dedup_key LIKE 'b7:%'")" "catalog.refresh queued"
wait_for scd_b1_main "SELECT state = 'queued' FROM public.job_status WHERE dedup_key = 'b7:one'" "main reads queued"
bj "UPDATE public.background_jobs SET lease_id = gen_random_uuid(), lease_generation = 1, leased_by = 'w', lease_expires_at = now() + interval '1 minute' WHERE dedup_key = 'b7:one'"
wait_for scd_b1_star "SELECT state = 'running' FROM public.job_status WHERE dedup_key = 'b7:one'" "star reads running after the lease"
bj "UPDATE public.background_jobs SET lease_expires_at = now() + interval '2 minutes' WHERE dedup_key = 'b7:one'"
want "a heartbeat writes nothing" "$(only core "SELECT n_tup_upd FROM pg_stat_user_tables WHERE relname = 'job_status'")" "$(only core "SELECT n_tup_upd FROM pg_stat_user_tables WHERE relname = 'job_status'")"
bj "UPDATE public.background_jobs SET lease_id = NULL, lease_generation = NULL, leased_by = NULL, lease_expires_at = NULL, available_at = now() + interval '10 minutes', last_error = 'boom' WHERE dedup_key = 'b7:one'"
wait_for scd_b1_main "SELECT state = 'retry_wait' AND available_at > now() + interval '9 minutes' FROM public.job_status WHERE dedup_key = 'b7:one'" "main reads retry_wait with its retry time"
bj "DELETE FROM public.background_jobs WHERE dedup_key = 'b7:one'"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.job_status WHERE dedup_key = 'b7:one')" "a finished job leaves no status"
sql -d scd_b1_core -c "ALTER TABLE public.background_jobs DISABLE TRIGGER zz_job_status_insert" -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload) VALUES ('00000000-0000-0000-0000-0000000b7022', 'playlists.observe_shadow', 'core_bulk', 'b7:late', '{}')" -c "ALTER TABLE public.background_jobs ENABLE ALWAYS TRIGGER zz_job_status_insert" > /dev/null
want "a job queued before the trigger has no status" "$(only core "SELECT count(*) FROM public.job_status WHERE dedup_key = 'b7:late'")" 0
want "the backfill adds it" "$(only core "SELECT sum(added) FROM public.scd_job_status_backfill(NULL, 1000)")" 1
wait_for scd_b1_main "SELECT state = 'queued' FROM public.job_status WHERE dedup_key = 'b7:late'" "main reads the backfilled status"
sql -d scd_b1_core -c "ALTER TABLE public.background_jobs DISABLE TRIGGER zz_job_status_delete" -c "DELETE FROM public.background_jobs WHERE dedup_key = 'b7:late'" -c "ALTER TABLE public.background_jobs ENABLE ALWAYS TRIGGER zz_job_status_delete" > /dev/null
want "the prune removes a status without a job" "$(only core "SELECT public.scd_job_status_prune(1000)")" 1
wait_for scd_b1_star "SELECT NOT EXISTS (SELECT 1 FROM public.job_status WHERE dedup_key = 'b7:late')" "star loses the pruned status"
only core "DELETE FROM public.background_jobs WHERE dedup_key LIKE 'b7:%'" > /dev/null
