say "S1 repair: an M row core purged comes back from the api's newer intent without the api's copy of core's columns"
ri_pk='{"user_id": "ri", "sc_track_id": "t1"}'
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('ri', 't1')"
calm
sql -d scd_b1_core -c "UPDATE public.user_likes_tracks SET progress = true, synced_at = '2026-01-01', liked_at = '2026-01-01', synced_for_v = wanted_v WHERE user_id = 'ri'"
wait_for scd_b1_main "SELECT progress AND synced_for_v > 0 FROM public.user_likes_tracks WHERE user_id = 'ri'" "main has core's result for ri"
ri_v="$(only core "SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = 'ri'")"
pause core core_from_main
sql -d scd_b1_main -c "UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'ri'"
want "core purges ri at the version it saw" "$(only core "SELECT public.scd_purge_intent('user_likes_tracks', '$ri_pk', $ri_v)")" t
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'user_likes_tracks' AND pk = '$ri_pk')" "main keeps its newer intent and queues it for resend"
resume core core_from_main
quiet
want "core takes the intent with its own columns at their defaults" \
  "$(only core "SELECT format('%s %s %s %s %s', wanted_state, progress, synced_at IS NULL, liked_at IS NULL, synced_for_v) FROM public.user_likes_tracks WHERE user_id = 'ri'")" \
  "f f t t 0"
pairs user_likes_tracks "user_id = 'ri'" "the repaired M row is equal on all nodes"

say "S1 lock order: core's drain and verify repair take the mirrors right before sync_queue, like the api tap (0292)"
fresh scd_b1_order scd_b1_tpl
only order "SELECT count(*) FROM public.scd_configure('core', 'core', 'split')" > /dev/null
sql -d scd_b1_order > /dev/null <<'SQL'
CREATE SCHEMA s1;
CREATE TABLE s1.drained (n bigserial PRIMARY KEY, tbl text NOT NULL);
CREATE FUNCTION s1.drained() RETURNS trigger LANGUAGE plpgsql AS $f$
BEGIN
    INSERT INTO s1.drained (tbl) VALUES (NEW.tbl);
    RETURN NULL;
END
$f$;
CREATE TRIGGER s1_drained AFTER INSERT ON public.repl_repair FOR EACH ROW EXECUTE FUNCTION s1.drained();
SET session_replication_role = replica;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000005b1', 'Order', 'order', 'sc');
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn) VALUES ('00000000-0000-0000-0000-0000000005b2', 'lo', 'like', 'urn:lo:1');
INSERT INTO public.track_comments (id, sc_track_id, user_urn, body) VALUES ('00000000-0000-0000-0000-0000000005b3', '42', 'soundcloud:users:1', 'order');
INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('lo', 't1');
INSERT INTO public.user_followings (user_id, target_user_urn) VALUES ('lo', 'soundcloud:users:2');
RESET session_replication_role;
INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('disliked_tracks', '{"id": "00000000-0000-0000-0000-0000000005b4"}', 0);
INSERT INTO public.repl_resend (tbl, pk) VALUES
    ('track_comments', '{"id": "00000000-0000-0000-0000-0000000005b3"}'),
    ('user_likes_tracks', '{"user_id": "lo", "sc_track_id": "t1"}'),
    ('sync_queue', '{"id": "00000000-0000-0000-0000-0000000005b2"}'),
    ('user_followings', '{"user_id": "lo", "target_user_urn": "soundcloud:users:2"}'),
    ('artists', '{"id": "00000000-0000-0000-0000-0000000005b1"}');
INSERT INTO public.repl_resend (tbl, pk)
SELECT 'repl_tombstones', jsonb_build_object('tbl', tbl, 'pk', pk, 'v', v) FROM public.repl_tombstones WHERE tbl = 'disliked_tracks';
INSERT INTO public.repl_checksums (node, tbl, range, hash, at) VALUES
    ('main', 'track_comments', 's1', 'x', now()),
    ('main', 'sync_queue', 's1', 'x', now()),
    ('main', 'user_likes_tracks', 's1', 'x', now()),
    ('star', 'artists', 's1', 'x', now()),
    ('star', 'user_followings', 's1', 'x', now());
SQL
want "the drain sends every queued row" "$(only order "SELECT sent || '/' || skipped || '/' || gone FROM public.scd_drain_resend_counts(500)")" "6/0/0"
want "the drain takes the mirrors right before sync_queue, the rest by live table" \
  "$(only order "SELECT string_agg(tbl, ' ' ORDER BY n) FROM s1.drained")" \
  "artists repl_tombstones user_followings user_likes_tracks sync_queue track_comments"
want "verify repair settles the answers in the same order" \
  "$(only order "SELECT string_agg(rel, ' ') FROM public.scd_verify_repair(false, 50)")" \
  "artists user_followings user_likes_tracks sync_queue track_comments"
sql -d postgres -c "DROP DATABASE scd_b1_order WITH (FORCE)" > /dev/null

say "S1 legacy job_status: each legacy node keeps job_status from its own and from replicated queue writes (0294)"
fresh scd_b1_legacy_main scd_b1_tpl
fresh scd_b1_legacy_star scd_b1_tpl
sql -d scd_b1_legacy_main -c "CREATE PUBLICATION s1_legacy_pub FOR TABLE public.background_jobs" > /dev/null
only legacy_main "SELECT pg_catalog.pg_create_logical_replication_slot('scd_b1_legacy_star_from_main', 'pgoutput')" > /dev/null
sql -d scd_b1_legacy_star -c "CREATE SUBSCRIPTION legacy_star_from_main CONNECTION 'host=$socket dbname=scd_b1_legacy_main user=scd' PUBLICATION s1_legacy_pub WITH (origin = none, create_slot = false, slot_name = 'scd_b1_legacy_star_from_main', copy_data = false)"
wait_for scd_b1_legacy_star "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'legacy_star_from_main' AND relid IS NULL AND pid IS NOT NULL)" "legacy star reads main"
sql -d scd_b1_legacy_main -c "INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload) VALUES
  ('00000000-0000-0000-0000-0000000005c1', 'catalog.refresh', 'core_bulk', 's1:one', '{}'),
  ('00000000-0000-0000-0000-0000000005c2', 'crawl.artist', 'core_bulk', 's1:two', '{}')"
want "main writes the status of its own enqueue, only for a read kind" "$(only legacy_main "SELECT string_agg(kind || ' ' || state, ',') FROM public.job_status")" "catalog.refresh queued"
wait_for scd_b1_legacy_star "SELECT coalesce(string_agg(kind || ' ' || state, ','), '') = 'catalog.refresh queued' FROM public.job_status" "star writes it from the replicated enqueue"
sql -d scd_b1_legacy_main -c "UPDATE public.background_jobs SET lease_id = gen_random_uuid(), lease_generation = 1, leased_by = 'w', lease_expires_at = now() + interval '1 minute' WHERE dedup_key = 's1:one'"
wait_for scd_b1_legacy_star "SELECT state = 'running' FROM public.job_status WHERE dedup_key = 's1:one'" "star reads running after main's lease"
sql -d scd_b1_legacy_main -c "UPDATE public.background_jobs SET lease_id = NULL, lease_generation = NULL, leased_by = NULL, lease_expires_at = NULL, last_error = 'boom', available_at = now() + interval '10 minutes' WHERE dedup_key = 's1:one'"
wait_for scd_b1_legacy_star "SELECT state = 'retry_wait' AND available_at > now() + interval '9 minutes' FROM public.job_status WHERE dedup_key = 's1:one'" "star reads retry_wait with its retry time"
sql -d scd_b1_legacy_main -c "DELETE FROM public.background_jobs WHERE dedup_key = 's1:one'"
wait_for scd_b1_legacy_star "SELECT NOT EXISTS (SELECT 1 FROM public.job_status)" "star drops the status of a finished job"
want "main drops it too" "$(only legacy_main "SELECT count(*) FROM public.job_status")" 0
want "no apply error on legacy star" "$(only legacy_star "SELECT apply_error_count FROM pg_catalog.pg_stat_subscription_stats WHERE subname = 'legacy_star_from_main'")" 0
sql -d scd_b1_legacy_star -c "ALTER SUBSCRIPTION legacy_star_from_main DISABLE" -c "ALTER SUBSCRIPTION legacy_star_from_main SET (slot_name = NONE)" -c "DROP SUBSCRIPTION legacy_star_from_main" > /dev/null
val postgres "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_legacy_star_from_main')" > /dev/null
sql -d postgres -c "DROP DATABASE scd_b1_legacy_star WITH (FORCE)" -c "DROP DATABASE scd_b1_legacy_main WITH (FORCE)" > /dev/null
