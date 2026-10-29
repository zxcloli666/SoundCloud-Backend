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
SELECT pg_temp.ok(wanted_v > 0 AND wanted_v % 16 = 2 AND result_v = 0, 'zz_ver: api insert bumps the intent version only')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO seen SELECT 'w1', wanted_v FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET last_read_at = now() WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_v = (SELECT v FROM seen WHERE k = 'w1') AND result_v = 0, 'zz_ver: a local column change keeps versions')
FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET wanted_state = false WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_v > (SELECT v FROM seen WHERE k = 'w1') AND wanted_v % 16 = 2 AND result_v = 0, 'zz_ver: an intent change bumps the intent version')
FROM public.user_likes_tracks WHERE user_id = 'u1';
INSERT INTO seen SELECT 'w2', wanted_v FROM public.user_likes_tracks WHERE user_id = 'u1';

SET session_replication_role = replica;
UPDATE public.user_likes_tracks SET wanted_state = true, wanted_v = 17, progress = true, result_v = 33 WHERE user_id = 'u1';
SELECT pg_temp.ok(NOT wanted_state AND wanted_v = (SELECT v FROM seen WHERE k = 'w2') AND progress AND result_v = 33,
    'zz_merge: per group, an older intent is dropped and a newer result is taken')
FROM public.user_likes_tracks WHERE user_id = 'u1';

INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v, progress, result_v, synced_at)
VALUES ('u1', 't1', true, 1, false, 49, '2026-01-01');
SELECT pg_temp.ok(count(*) = 1 AND bool_and(NOT wanted_state AND NOT progress AND result_v = 49 AND synced_at = '2026-01-01'),
    'zz_merge: an insert on an existing key becomes a per-group merge')
FROM public.user_likes_tracks WHERE user_id = 'u1';

UPDATE public.user_likes_tracks SET wanted_state = true, wanted_v = (SELECT v FROM seen WHERE k = 'w2') + 16 WHERE user_id = 'u1';
SELECT pg_temp.ok(wanted_state AND wanted_v = (SELECT v FROM seen WHERE k = 'w2') + 16, 'zz_merge: a newer intent version wins')
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
SELECT pg_temp.ok(count(*) = 1, 'zz_tomb_check: a re-create newer than the tombstone is applied')
FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000000f1';

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
SET search_path = public;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b2', 'B', 'b', 'sc');
INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms, primary_artist_id, last_read_at)
VALUES ('00000000-0000-0000-0000-0000000000d1', 'urn:t:1', 'sc1', 'T', 't', 1000, '00000000-0000-0000-0000-0000000000b2', '2026-01-02');
INSERT INTO public.track_artists (track_id, artist_id, role, source)
VALUES ('00000000-0000-0000-0000-0000000000d1', '00000000-0000-0000-0000-0000000000b2', 'primary', 'sc');
INSERT INTO public.cmd_pending (tbl, pk, node, id)
VALUES ('artists', '{"id": "00000000-0000-0000-0000-0000000000b3"}', 'main', '00000000-0000-0000-0000-000000000002');
SET search_path = '';
RESET scd.cmd;

SET session_replication_role = replica;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b3', 'P', 'p', 'sc');
SELECT pg_temp.ok(count(*) = 0, 'zz_owner: a core row for a key with a pending command is skipped')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';
DELETE FROM public.cmd_pending;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b3', 'P', 'p', 'sc');
SELECT pg_temp.ok(count(*) = 1, 'zz_owner: the same row applies once the command is acked')
FROM public.artists WHERE id = '00000000-0000-0000-0000-0000000000b3';

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
SET search_path = public;
INSERT INTO public.artists (id, name, normalized_name, source) VALUES ('00000000-0000-0000-0000-0000000000b5', 'R', 'r', 'sc');
UPDATE public.tracks SET primary_artist_id = '00000000-0000-0000-0000-0000000000b5' WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT public.scd_purge('artists', '{"id": "00000000-0000-0000-0000-0000000000b5"}');
SET search_path = '';
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
SET search_path = public;
UPDATE public.tracks SET last_read_at = now() WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT pg_temp.ok(h = public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}'), 'scd_row_hash: local columns are not hashed') FROM hashes;
UPDATE public.tracks SET title = 'changed' WHERE id = '00000000-0000-0000-0000-0000000000d1';
SELECT pg_temp.ok(h <> public.scd_row_hash('tracks', '{"id": "00000000-0000-0000-0000-0000000000d1"}'), 'scd_row_hash: a published column change changes the hash') FROM hashes;
SET search_path = '';
RESET scd.cmd;

SELECT pg_temp.ok(count(*) = 0, 'scd_publication_lists: every list has the primary key')
FROM public.scd_publication_lists() l
JOIN pg_catalog.pg_index i ON i.indrelid = ('public.' || l.tbl)::regclass AND i.indisprimary
JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
WHERE NOT a.attname::text = ANY (l.cols);

SELECT 'smoke ok' AS result;
