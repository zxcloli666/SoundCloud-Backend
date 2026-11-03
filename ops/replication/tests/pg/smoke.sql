SET search_path = '';
SET client_min_messages = notice;
SET scd.mode = 'split';
SET scd.role = 'api';
SET scd.node = 'main';

CREATE FUNCTION pg_temp.ok(cond boolean, name text) RETURNS void LANGUAGE plpgsql AS $f$
BEGIN
    IF cond IS NOT TRUE THEN
        RAISE EXCEPTION 'smoke failed: %', name;
    END IF;
    RAISE NOTICE 'ok %', name;
END
$f$;

CREATE FUNCTION pg_temp.n(c text, t text) RETURNS bigint LANGUAGE sql AS $f$
SELECT n FROM public.scd_counters WHERE counter = c AND tbl = t
$f$;

CREATE TEMP TABLE seen (k text PRIMARY KEY, v bigint);

INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('u1', 't1');
SELECT pg_temp.ok(wanted_v > 0 AND wanted_v % 16 = 2, 'zz_ver: api insert bumps the intent version')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO seen SELECT 'w1', wanted_v FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET last_read_at = now() WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_v = (SELECT v FROM seen WHERE k = 'w1'), 'zz_ver: a local column change keeps versions')
FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_v > (SELECT v FROM seen WHERE k = 'w1') AND wanted_v % 16 = 2, 'zz_ver: an intent change bumps the intent version')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO seen SELECT 'w2', wanted_v FROM public.user_likes_tracks WHERE user_id = 'u1';

SET session_replication_role = replica;
UPDATE public.user_likes_tracks SET wanted_state = true, wanted_v = 17, progress = true WHERE user_id = 'u1';
SELECT pg_temp.ok(NOT wanted_state AND wanted_v = (SELECT v FROM seen WHERE k = 'w2') AND progress,
    'zz_merge on api: an older intent is dropped, core''s result is taken unconditionally')
FROM public.user_likes_tracks WHERE user_id = 'u1';

INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v, progress, synced_at)
VALUES ('u1', 't1', true, 1, false, '2026-01-01');
SELECT pg_temp.ok(count(*) = 1 AND bool_and(NOT wanted_state AND NOT progress AND synced_at = '2026-01-01'),
    'zz_merge on api: an insert on an existing key keeps the newer intent and takes core''s result')
FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET wanted_state = true WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_state, 'zz_merge on api: a tie takes the incoming row')
FROM public.user_likes_tracks WHERE user_id = 'u1';

SET scd.role = 'core';
UPDATE public.user_likes_tracks SET wanted_state = false, progress = true, synced_at = '2026-02-02' WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_state AND NOT progress AND synced_at = '2026-01-01',
    'zz_merge on core: a tie keeps the local intent, api values never reach core''s result')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v, progress, synced_at)
VALUES ('u1', 't1', false, (SELECT v FROM seen WHERE k = 'w2') + 16, true, '2026-03-03');
SELECT pg_temp.ok(NOT wanted_state AND wanted_v = (SELECT v FROM seen WHERE k = 'w2') + 16 AND NOT progress AND synced_at = '2026-01-01',
    'zz_merge on core: a newer api insert on an existing key takes the intent and keeps core''s result')
FROM public.user_likes_tracks WHERE user_id = 'u1';
SET scd.role = 'api';

INSERT INTO seen SELECT 'c1', extract(epoch FROM created_at)::bigint FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v, created_at)
VALUES ('u1', 't1', true, ((SELECT v FROM seen WHERE k = 'w2') & ~15) + 2, '2020-01-01');
SELECT pg_temp.ok(created_at > '2021-01-01', 'zz_merge D2: an older echo of this node''s own insert does not move created_at')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v, created_at)
VALUES ('u1', 't1', true, ((SELECT v FROM seen WHERE k = 'w2') & ~15) + 3, '2020-01-01');
SELECT pg_temp.ok(created_at = '2020-01-01' AND NOT wanted_state,
    'zz_merge D2: a row born on the other api node merges created_at with least, the older intent stays')
FROM public.user_likes_tracks WHERE user_id = 'u1';
UPDATE public.user_likes_tracks SET created_at = '2026-05-05', wanted_v = wanted_v + 16 WHERE user_id = 'u1';
SELECT pg_temp.ok(created_at = '2026-05-05', 'zz_merge D2: a replicated update of created_at (re-like) is taken as is')
FROM public.user_likes_tracks WHERE user_id = 'u1';

SET scd.mode = 'legacy';
DO $d$
BEGIN
    INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_v) VALUES ('u1', 't1', 99999999999);
    RAISE EXCEPTION 'smoke failed: legacy mode merged';
EXCEPTION WHEN unique_violation THEN
    RAISE NOTICE 'ok zz_merge: off in legacy mode';
END
$d$;
SET scd.mode = 'split';

SET session_replication_role = origin;
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, payload)
VALUES ('00000000-0000-0000-0000-00000000000a', 'u1', 'like', 'urn:x', '{"n": 1}');
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn)
VALUES ('00000000-0000-0000-0000-0000000000c1', 'u1', 'comment', 'urn:x');
SET session_replication_role = replica;
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, payload, intent_v)
VALUES ('00000000-0000-0000-0000-00000000000b', 'u1', 'like', 'urn:x', '{"n": 2}', public.scd_hlc(0) + 1600000000);
SELECT pg_temp.ok(count(*) = 1 AND bool_and(id = '00000000-0000-0000-0000-00000000000a' AND payload = '{"n": 2}'),
    'zz_merge: an R6 natural-key match with another id merges into the local row')
FROM public.sync_queue WHERE action_type = 'like';
SELECT pg_temp.ok(pg_temp.n('conflict', 'sync_queue') = 1, 'zz_merge: the R6 id conflict is counted');
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn)
VALUES ('00000000-0000-0000-0000-0000000000c2', 'u1', 'comment', 'urn:x');
SELECT pg_temp.ok(count(*) = 2 AND pg_temp.n('conflict', 'sync_queue') = 1, 'zz_merge: rows outside the partial unique index are not merged')
FROM public.sync_queue WHERE action_type = 'comment';

SET session_replication_role = origin;
INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight)
VALUES ('00000000-0000-0000-0000-0000000000e1', 'u1', 't1', 'play', 1);
SET session_replication_role = replica;
INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight)
VALUES ('00000000-0000-0000-0000-0000000000e1', 'u1', 't1', 'play', 7);
SELECT pg_temp.ok(count(*) = 1 AND bool_and(weight = 1), 'zz_skip: an insert-only row with an existing key is skipped')
FROM public.user_events WHERE sc_user_id = 'u1';

SET session_replication_role = origin;
INSERT INTO public.sessions (id) VALUES ('00000000-0000-0000-0000-0000000000f1');
INSERT INTO seen SELECT 's1', row_v FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f1';
DELETE FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f1';
SELECT pg_temp.ok(v > (SELECT v FROM seen WHERE k = 's1'), 'zz_tomb: a delete writes a tombstone newer than the row')
FROM public.repl_tombstones WHERE tbl = 'sessions' AND pk = '{"id": "00000000-0000-0000-0000-0000000000f1"}';
SET session_replication_role = replica;
INSERT INTO public.sessions (id, row_v) VALUES ('00000000-0000-0000-0000-0000000000f1', (SELECT v FROM seen WHERE k = 's1'));
SELECT pg_temp.ok(count(*) = 0, 'zz_tomb_check: a late echo insert of a deleted row is blocked')
FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f1';
INSERT INTO public.sessions (id, row_v) VALUES ('00000000-0000-0000-0000-0000000000f1', public.scd_hlc(0) + 1600000000);
SELECT pg_temp.ok(count(*) = 0, 'zz_tomb_check: a random uuid key stays deleted even with a newer version')
FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f1';
SET session_replication_role = origin;
INSERT INTO public.sessions (id) VALUES ('00000000-0000-0000-0000-0000000000f2');
SET session_replication_role = replica;
INSERT INTO public.repl_tombstones (tbl, pk, v) VALUES ('sessions', '{"id": "00000000-0000-0000-0000-0000000000f2"}', 18);
SELECT pg_temp.ok(count(*) = 0 AND (SELECT count(*) FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000f2') = 1,
    'zz_erase: a tombstone deletes a live delete-wins row whatever its version, and is kept')
FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f2';

SET session_replication_role = origin;
INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-0000000000d1', 'u9', 't1');
INSERT INTO seen SELECT 'd1', row_v FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
DELETE FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
SET session_replication_role = replica;
INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, row_v)
VALUES ('00000000-0000-0000-0000-0000000000d1', 'u9', 't1', (SELECT v FROM seen WHERE k = 'd1'));
SELECT pg_temp.ok(count(*) = 0 AND (SELECT count(*) FROM public.repl_resend WHERE tbl = 'repl_tombstones') = 1,
    'zz_merge: an echo older than the tombstone is blocked and the tombstone is queued for resend')
FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id, row_v)
VALUES ('00000000-0000-0000-0000-0000000000d1', 'u9', 't1', public.scd_hlc(0) + 1600000000);
SELECT pg_temp.ok(count(*) = 1, 'zz_merge: a re-create newer than the tombstone is applied')
FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
DELETE FROM public.repl_resend;
SET session_replication_role = origin;
DELETE FROM public.disliked_tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT pg_temp.ok(count(*) = 2 AND count(DISTINCT v) = 2 AND max(v) > (SELECT v FROM seen WHERE k = 'd1'),
    'zz_tomb: a second delete of the key adds a newer tombstone row')
FROM public.repl_tombstones WHERE tbl = 'disliked_tracks' AND pk = '{"id": "00000000-0000-0000-0000-0000000000d1"}';
DO $d$
BEGIN
    UPDATE public.repl_tombstones SET at = now() WHERE tbl = 'disliked_tracks';
    RAISE EXCEPTION 'smoke failed: a tombstone update went through';
EXCEPTION WHEN feature_not_supported THEN
    RAISE NOTICE 'ok zz_ver: a tombstone update is refused, rows are immutable';
END
$d$;
INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES
    ('00000000-0000-0000-0000-0000000000d2', 'u9', 't2'), ('00000000-0000-0000-0000-0000000000d3', 'u9', 't3');
SET session_replication_role = replica;
INSERT INTO public.repl_tombstones (tbl, pk, v)
SELECT 'disliked_tracks', jsonb_build_object('id', id), CASE sc_track_id WHEN 't2' THEN row_v + 16 ELSE row_v - 16 END
FROM public.disliked_tracks WHERE id IN ('00000000-0000-0000-0000-0000000000d2', '00000000-0000-0000-0000-0000000000d3');
SELECT pg_temp.ok(array_agg(sc_track_id) = ARRAY['t3']
    AND (SELECT array_agg(pk ->> 'id') FROM public.repl_resend WHERE tbl = 'disliked_tracks') = ARRAY['00000000-0000-0000-0000-0000000000d3']
    AND (SELECT count(*) FROM public.repl_tombstones WHERE pk ->> 'id' IN ('00000000-0000-0000-0000-0000000000d2', '00000000-0000-0000-0000-0000000000d3')) = 2,
    'zz_erase: a tombstone deletes the live row it covers; a newer row stays and is queued for resend')
FROM public.disliked_tracks WHERE id IN ('00000000-0000-0000-0000-0000000000d2', '00000000-0000-0000-0000-0000000000d3');
INSERT INTO public.repl_tombstones (tbl, pk, v)
SELECT tbl, pk, v FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000d2';
SELECT pg_temp.ok(count(*) = 1, 'zz_erase: an echo of a tombstone the node has is a no-op')
FROM public.repl_tombstones WHERE pk ->> 'id' = '00000000-0000-0000-0000-0000000000d2';
DELETE FROM public.repl_resend;

SET session_replication_role = origin;
INSERT INTO public.listening_history (id, soundcloud_user_id, sc_track_id, title, artist_name, duration)
VALUES ('00000000-0000-0000-0000-0000000000a1', 'u1', 't1', 'x', 'y', 1);
DELETE FROM public.listening_history WHERE id = '00000000-0000-0000-0000-0000000000a1';
SET session_replication_role = replica;
INSERT INTO public.listening_history (id, soundcloud_user_id, sc_track_id, title, artist_name, duration)
VALUES ('00000000-0000-0000-0000-0000000000a1', 'u1', 't1', 'x', 'y', 1);
SELECT pg_temp.ok(count(*) = 0, 'zz_tomb_check: an unversioned random-id row stays deleted')
FROM public.listening_history WHERE id = '00000000-0000-0000-0000-0000000000a1';

SET session_replication_role = origin;
SELECT pg_temp.ok(pg_temp.n('guard', 'artists') = 0, 'zz_guard: counter starts at 0');
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b1', 'A', 'a', 'sc');
SELECT pg_temp.ok(pg_temp.n('guard', 'artists') = 1, 'zz_guard: an unwrapped C write on an api node is counted');
SET scd.cmd = '00000000-0000-0000-0000-000000000001';
UPDATE public.artists SET name = 'A2' WHERE id = '00000000-0000-0000-0000-0000000000b1';
RESET scd.cmd;
SELECT pg_temp.ok(pg_temp.n('guard', 'artists') = 1, 'zz_guard: a write inside a command is not counted');
SET scd.guard = 'enforce';
DO $d$
BEGIN
    UPDATE public.artists SET name = 'A3' WHERE id = '00000000-0000-0000-0000-0000000000b1';
    RAISE EXCEPTION 'smoke failed: enforce let the write through';
EXCEPTION WHEN insufficient_privilege THEN
    RAISE NOTICE 'ok zz_guard: enforce mode refuses the write';
END
$d$;
RESET scd.guard;
SET scd.writer = 'migration';
UPDATE public.artists SET name = 'A2' WHERE id = '00000000-0000-0000-0000-0000000000b1';
RESET scd.writer;
SELECT pg_temp.ok(pg_temp.n('guard', 'artists') = 2, 'zz_guard: the migration writer is exempt (enforce attempt counted once)');
UPDATE public.user_likes_tracks SET progress = false WHERE user_id = 'u1';
SELECT pg_temp.ok(pg_temp.n('guard_result', 'user_likes_tracks') = 1, 'zz_guard_result: an api write of a core-owned M column is counted');
SET scd.role = 'core';
UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'u1';
SELECT pg_temp.ok(pg_temp.n('guard_intent', 'user_likes_tracks') = 1, 'zz_guard_intent: a core write of an api-owned M column is counted');
SET scd.writer = 'api';
UPDATE public.user_likes_tracks SET wanted_state = true WHERE user_id = 'u1';
RESET scd.writer;
SELECT pg_temp.ok(pg_temp.n('guard_intent', 'user_likes_tracks') = 1, 'zz_guard_intent: the core-local api writer is exempt');
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b9', 'C', 'core-only', 'sc');
SELECT pg_temp.ok(pg_temp.n('guard', 'artists') = 2, 'zz_guard: C writes on core are not counted');
SET scd.role = 'api';

SET scd.cmd = '00000000-0000-0000-0000-000000000002';
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b2', 'B', 'b', 'sc');
INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms, primary_artist_id, last_read_at)
VALUES ('00000000-0000-0000-0000-0000000000d1', 'urn:t:1', 'sc1', 'T', 't', 1000, '00000000-0000-0000-0000-0000000000b2', '2026-01-02');
INSERT INTO public.track_artists (track_id, artist_id, role, source)
VALUES ('00000000-0000-0000-0000-0000000000d1', '00000000-0000-0000-0000-0000000000b2', 'primary', 'sc');
INSERT INTO public.cmd_pending (tbl, pk, node, id)
VALUES ('artists', '{"id": "00000000-0000-0000-0000-0000000000b3"}', 'main', '00000000-0000-0000-0000-000000000002');
RESET scd.cmd;

SET session_replication_role = replica;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b3', 'P', 'p', 'sc');
SELECT pg_temp.ok(count(*) = 0, 'zz_owner: a core row for a key with a pending command is skipped')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
DELETE FROM public.cmd_pending;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b3', 'P', 'p', 'sc');
SELECT pg_temp.ok(count(*) = 1, 'zz_owner: the same row applies once the command is acked')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
INSERT INTO public.cmd_pending (tbl, pk, node, id)
VALUES ('artists', '{"id": "00000000-0000-0000-0000-0000000000b3"}', 'main', '00000000-0000-0000-0000-000000000005');
UPDATE public.artists SET name = 'P core', has_sc_account = true WHERE id = '00000000-0000-0000-0000-0000000000b3';
SELECT pg_temp.ok(name = 'P' AND has_sc_account, 'zz_owner: a pending command keeps the optimistic row but takes core columns from core')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
UPDATE public.artists SET has_sc_account = false WHERE id = '00000000-0000-0000-0000-0000000000b3';
INSERT INTO public.artists (id, name, normalized_name, source, has_sc_account) VALUES ('00000000-0000-0000-0000-0000000000b3', 'P core', 'p', 'sc', true);
SELECT pg_temp.ok(name = 'P' AND has_sc_account, 'zz_owner: a pending command keeps the optimistic row; a core insert still sets core columns')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
SET session_replication_role = origin;
SET scd.cmd = '00000000-0000-0000-0000-000000000005';
UPDATE public.artists SET has_sc_account = false, name = 'P api' WHERE id = '00000000-0000-0000-0000-0000000000b3';
RESET scd.cmd;
SELECT pg_temp.ok(name = 'P api' AND has_sc_account AND pg_temp.n('core_col', 'artists') = 1,
    'zz_core_col: an api write of a core column keeps core''s value and is counted')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
CREATE TEMP TABLE core_hash AS SELECT public.scd_row_hash('artists', '{"id": "00000000-0000-0000-0000-0000000000b3"}') AS h;
SET session_replication_role = replica;
UPDATE public.artists SET has_sc_account = false WHERE id = '00000000-0000-0000-0000-0000000000b3';
SELECT pg_temp.ok(h = public.scd_row_hash('artists', '{"id": "00000000-0000-0000-0000-0000000000b3"}'), 'scd_row_hash: core columns are not hashed')
FROM core_hash;
DELETE FROM public.cmd_pending;
INSERT INTO public.artist_sc_accounts (artist_id, sc_user_id, role, source, verified, created_at, updated_at)
VALUES ('00000000-0000-0000-0000-0000000000b3', 'scu-same', 'main', 'sc', true, '2026-01-03', '2026-01-03');
INSERT INTO public.artist_sc_accounts (artist_id, sc_user_id, role, source, verified, created_at, updated_at)
VALUES ('00000000-0000-0000-0000-0000000000b3', 'scu-same', 'main', 'sc', true, '2026-01-03', '2026-01-03');
SELECT pg_temp.ok(count(*) = 1, 'zz_owner: a core row equal to the local row applies as a no-op')
FROM public.artist_sc_accounts WHERE artist_id = '00000000-0000-0000-0000-0000000000b3';

INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
VALUES ('00000000-0000-0000-0000-0000000000d1', 'urn:t:1', 'sc1', 'Core title', 'core title', 2000);
SELECT pg_temp.ok(count(*) = 1 AND bool_and(title = 'Core title' AND primary_artist_id IS NULL AND last_read_at = '2026-01-02'),
    'zz_owner: core''s insert overwrites the optimistic row and keeps local columns')
FROM public.tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';

INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b4', 'B core', 'b', 'sc');
SELECT pg_temp.ok(
    (SELECT count(*) FROM public.artists WHERE normalized_name = 'b') = 1
    AND EXISTS (SELECT 1 FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b4')
    AND NOT EXISTS (SELECT 1 FROM public.track_artists WHERE artist_id = '00000000-0000-0000-0000-0000000000b2'),
    'zz_owner: core wins a unique key; the local row and its cascade children are purged');

SET session_replication_role = origin;
SET scd.cmd = '00000000-0000-0000-0000-000000000003';
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b5', 'R', 'r', 'sc');
UPDATE public.tracks SET primary_artist_id = '00000000-0000-0000-0000-0000000000b5' WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT public.scd_purge('artists', '{"id": "00000000-0000-0000-0000-0000000000b5"}');
RESET scd.cmd;
SELECT pg_temp.ok(primary_artist_id IS NULL, 'scd_purge: a NO ACTION nullable child reference is cleared before the parent goes')
FROM public.tracks WHERE id = '00000000-0000-0000-0000-0000000000d1';
DO $d$
BEGIN
    PERFORM public.scd_purge('nope', '{}');
    RAISE EXCEPTION 'smoke failed: unknown table accepted';
EXCEPTION WHEN raise_exception THEN
    IF SQLERRM NOT LIKE 'public.scd_purge: unknown table%' THEN
        RAISE;
    END IF;
    RAISE NOTICE 'ok scd_purge: unknown table raises';
END
$d$;

SET session_replication_role = origin;
SET scd.writer = 'migration';
INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('u2', 't1'), ('u3', 't1'), ('u4', 't1');
RESET scd.writer;
INSERT INTO public.repl_purges (tbl, pk, purged_v)
SELECT 'user_likes_tracks', jsonb_build_object('user_id', user_id, 'sc_track_id', sc_track_id),
    CASE user_id WHEN 'u2' THEN wanted_v ELSE wanted_v - 16 END
FROM public.user_likes_tracks WHERE user_id IN ('u2', 'u3');
SET session_replication_role = replica;
DELETE FROM public.user_likes_tracks WHERE user_id IN ('u2', 'u3', 'u4');
SELECT pg_temp.ok(
    NOT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'u2')
    AND (SELECT count(*) FROM public.user_likes_tracks WHERE user_id IN ('u3', 'u4')) = 2
    AND (SELECT count(*) FROM public.repl_resend WHERE tbl = 'user_likes_tracks') = 2
    AND pg_temp.n('resend', 'user_likes_tracks') = 2,
    'zz_purge_guard: a purge covered by its receipt applies; newer or unreceipted intent is kept and queued for resend');
INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_v)
SELECT 'u2', 't1', purged_v FROM public.repl_purges WHERE tbl = 'user_likes_tracks' AND pk ->> 'user_id' = 'u2';
SELECT pg_temp.ok(NOT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'u2') AND pg_temp.n('purge_block', 'user_likes_tracks') = 1,
    'zz_merge: an M row at or below its purge is refused and counted');
CREATE TEMP TABLE u3_before AS SELECT wanted_v FROM public.user_likes_tracks WHERE user_id = 'u3';
UPDATE public.user_likes_tracks t SET wanted_v = p.purged_v, wanted_state = false
FROM public.repl_purges p WHERE p.tbl = 'user_likes_tracks' AND p.pk ->> 'user_id' = 'u3' AND t.user_id = 'u3';
SELECT pg_temp.ok(t.wanted_v = b.wanted_v AND t.wanted_state AND pg_temp.n('purge_block', 'user_likes_tracks') = 2,
    'zz_merge: an M update at or below its purge is refused and counted')
FROM public.user_likes_tracks t, u3_before b WHERE t.user_id = 'u3';
INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_v)
SELECT 'u2', 't1', purged_v + 16 FROM public.repl_purges WHERE tbl = 'user_likes_tracks' AND pk ->> 'user_id' = 'u2';
SELECT pg_temp.ok(count(*) = 1 AND pg_temp.n('purge_block', 'user_likes_tracks') = 2, 'zz_merge: an M row above its purge applies')
FROM public.user_likes_tracks WHERE user_id = 'u2';
SET session_replication_role = origin;
INSERT INTO public.repl_purges (tbl, pk, purged_v)
VALUES ('user_likes_tracks', '{"user_id": "u6", "sc_track_id": "t1"}', public.scd_hlc(0) + (3600000000::bigint << 4));
INSERT INTO public.user_likes_tracks (user_id, sc_track_id) VALUES ('u6', 't1');
SELECT pg_temp.ok(t.wanted_v > p.purged_v, 'zz_ver: a like after a purge ahead of the clock starts above the purge')
FROM public.user_likes_tracks t, public.repl_purges p WHERE t.user_id = 'u6' AND p.pk ->> 'user_id' = 'u6';
UPDATE public.repl_purges SET purged_v = purged_v + (7200000000::bigint << 4) WHERE pk ->> 'user_id' = 'u6';
UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'u6';
SELECT pg_temp.ok(t.wanted_v > p.purged_v, 'zz_ver: an intent change after a purge ahead of the clock goes above the purge')
FROM public.user_likes_tracks t, public.repl_purges p WHERE t.user_id = 'u6' AND p.pk ->> 'user_id' = 'u6';
SET session_replication_role = replica;

SET scd.role = 'core';
SET session_replication_role = origin;
INSERT INTO public.sc_track_counters (sc_track_id, fetched_at) VALUES ('k1', '2026-01-02');
SET session_replication_role = replica;
INSERT INTO public.sc_track_counters (sc_track_id, fetched_at) VALUES ('k1', '2026-01-01');
SELECT pg_temp.ok(fetched_at = '2026-01-02', 'zz_max: an older register insert is ignored') FROM public.sc_track_counters WHERE sc_track_id = 'k1';
INSERT INTO public.sc_track_counters (sc_track_id, fetched_at) VALUES ('k1', '2026-01-03');
SELECT pg_temp.ok(fetched_at = '2026-01-03', 'zz_max: a newer register insert wins') FROM public.sc_track_counters WHERE sc_track_id = 'k1';
UPDATE public.sc_track_counters SET fetched_at = '2026-01-01' WHERE sc_track_id = 'k1';
SELECT pg_temp.ok(fetched_at = '2026-01-03', 'zz_max: an older register update is ignored') FROM public.sc_track_counters WHERE sc_track_id = 'k1';
SET scd.role = 'api';

INSERT INTO seen SELECT 'x1', xmin::text::bigint FROM public.sc_track_counters WHERE sc_track_id = 'k1';
UPDATE public.sc_track_counters SET fetched_at = fetched_at WHERE sc_track_id = 'k1';
SELECT pg_temp.ok(xmin::text::bigint = (SELECT v FROM seen WHERE k = 'x1'), 'zzz_suppress: a no-op replica update writes no row version')
FROM public.sc_track_counters WHERE sc_track_id = 'k1';

SET session_replication_role = origin;
SET timezone = 'Asia/Tokyo';
SET extra_float_digits = 0;
CREATE TEMP TABLE hashes AS SELECT public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}') AS h;
SET timezone = 'UTC';
RESET extra_float_digits;
SELECT pg_temp.ok(h IS NOT NULL AND h = public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}'),
    'scd_row_hash: the hash does not depend on session settings') FROM hashes;
SET scd.cmd = '00000000-0000-0000-0000-000000000004';
UPDATE public.tracks SET last_read_at = now() WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT pg_temp.ok(h = public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}'), 'scd_row_hash: local columns are not hashed') FROM hashes;
UPDATE public.tracks SET title = 'changed' WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT pg_temp.ok(h <> public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}'), 'scd_row_hash: a published column change changes the hash') FROM hashes;
RESET scd.cmd;

SELECT pg_temp.ok(count(*) = 0, 'scd_publication_lists: every list has the primary key')
FROM public.scd_publication_lists() l
JOIN pg_catalog.pg_index i ON i.indrelid = ('public.' || l.tbl)::regclass AND i.indisprimary
JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
WHERE NOT a.attname::text = ANY (l.cols);

SELECT pg_temp.ok(public.uuid_generate_v5('6ba7b810-9dad-11d1-80b4-00c04fd430c8', 'python.org') = '886313e1-3b8a-5372-9b90-0c9aee199e5d',
    'uuid_generate_v5: the RFC 4122 version 5 vector (DNS namespace, python.org)');
SELECT pg_temp.ok(public.scd_uuid('like', 'u1', 't1') = '564fd801-7574-52a6-be4c-1bd16e134126'
    AND public.scd_uuid('k', E'\u00e9') = 'b4d9d12d-63a8-5d3e-a365-9995907be3b5',
    'scd_uuid: version 5, fixed namespace, unit separator and UTF-8 bytes');
SELECT pg_temp.ok(to_regprocedure('public.scd_uuid_v3(uuid, text)') IS NULL, 'scd_uuid: the version 3 helper is gone, one id scheme');
SELECT pg_temp.ok(public.scd_uuid('like', 'u1', NULL) IS NULL AND public.scd_uuid(NULL, 'u1') IS NULL
    AND public.scd_uuid('like', VARIADIC ARRAY[]::text[]) IS NULL, 'scd_uuid: a missing key part gives NULL');

SET scd.node = 'star';
CREATE TEMP TABLE clock AS SELECT public.scd_hlc(0) AS v1, public.scd_hlc(4611686018427387904) AS far;
SELECT pg_temp.ok(v1 & 15 = 3 AND public.scd_hlc(v1) > v1 AND far = 4611686018427387904 + 16 + 3
    AND abs(public.scd_hlc_offset_ms(v1)) < 5000, 'scd_hlc: node ordinal in the low bits, strictly above prev, wall clock otherwise')
FROM clock;
SET scd.node = 'nowhere';
SELECT pg_temp.ok(public.scd_hlc(0) IS NULL, 'scd_hlc: an unknown node gives NULL, so a version write fails');
SET scd.node = 'main';

SELECT 'smoke ok' AS result;
