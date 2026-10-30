CREATE SCHEMA wt_test;

CREATE FUNCTION wt_test.table_hashes()
RETURNS TABLE (tbl text, hash text)
LANGUAGE plpgsql
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
BEGIN
    FOR tbl IN
        SELECT c.relname::text FROM pg_catalog.pg_class c
        WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p')
          AND public.scd_table_class(c.relname) IN ('C', 'P', 'M', 'A', 'K')
        ORDER BY 1
    LOOP
        EXECUTE format('SELECT md5(coalesce(string_agg(r::text, E''\n'' ORDER BY r::text), '''')) FROM public.%I r', tbl) INTO hash;
        RETURN NEXT;
    END LOOP;
END
$f$;

ALTER FUNCTION public.scd_kinds() RENAME TO scd_kinds_registry;

CREATE FUNCTION public.scd_kinds()
RETURNS TABLE (kind text, optimistic boolean)
LANGUAGE sql STABLE
AS $f$
SELECT r.kind, r.optimistic FROM public.scd_kinds_registry() r
UNION ALL
VALUES ('test.title', true), ('test.playlist', true), ('test.revive', false), ('test.race', true), ('test.stamp', true), ('test.array', true)
$f$;

CREATE FUNCTION public.scd_cmd_test_title(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    UPDATE public.tracks SET title = args ->> 'title', title_normalized = lower(args ->> 'title')
    WHERE urn = args ->> 'urn' RETURNING id INTO t;
    RETURN jsonb_build_object(
        'touched', CASE WHEN t IS NULL THEN '[]'::jsonb
            ELSE jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))) END,
        'output', jsonb_build_object('id', t, 'title', args ->> 'title'));
END
$f$;

CREATE FUNCTION public.scd_cmd_test_playlist(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    stamp timestamptz := (args ->> 'observed_at')::timestamptz;
    fresh boolean;
BEGIN
    INSERT INTO public.playlists AS p (urn, sc_playlist_id, title, title_normalized, sc_synced_at, created_at, updated_at)
    VALUES (args ->> 'urn', args ->> 'sc_id', args ->> 'title', lower(args ->> 'title'), stamp, stamp, stamp)
    ON CONFLICT (urn) DO UPDATE SET title = EXCLUDED.title, title_normalized = EXCLUDED.title_normalized, updated_at = EXCLUDED.updated_at
    RETURNING (p.xmax = 0) INTO fresh;
    INSERT INTO public.playlist_membership_state (playlist_urn, updated_at) VALUES (args ->> 'urn', stamp) ON CONFLICT DO NOTHING;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(
            jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', args ->> 'urn')),
            jsonb_build_object('tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', args ->> 'urn'))),
        'output', jsonb_build_object('was_new', fresh));
END
$f$;

CREATE FUNCTION public.scd_cmd_test_revive(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    n integer;
BEGIN
    UPDATE public.sync_queue SET dead = false, retry_count = 0 WHERE id = (args ->> 'id')::uuid AND dead;
    GET DIAGNOSTICS n = ROW_COUNT;
    RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', jsonb_build_object('revived', n));
END
$f$;

CREATE FUNCTION public.scd_cmd_test_stamp(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    UPDATE public.tracks SET title = clock_timestamp()::text, title_normalized = '' WHERE urn = args ->> 'urn' RETURNING id INTO t;
    RETURN jsonb_build_object('touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))));
END
$f$;

CREATE FUNCTION public.scd_cmd_test_array(args jsonb) RETURNS jsonb LANGUAGE sql AS $f$
SELECT '[]'::jsonb
$f$;

CREATE TABLE wt_test.race (id uuid, urn text, note text);

CREATE FUNCTION wt_test.hold(id uuid) RETURNS uuid LANGUAGE plpgsql IMMUTABLE AS $f$
BEGIN
    IF current_setting('wt.hold', true) = 'on' THEN
        PERFORM pg_advisory_lock_shared(7424202);
        PERFORM pg_advisory_unlock_shared(7424202);
    END IF;
    RETURN id;
END
$f$;

CREATE INDEX race_hold ON wt_test.race (wt_test.hold(id));
ALTER TABLE wt_test.race ADD PRIMARY KEY (id);
ALTER TABLE wt_test.race ADD CONSTRAINT race_urn_key UNIQUE (urn);

CREATE FUNCTION public.scd_cmd_test_race(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    fresh boolean;
BEGIN
    INSERT INTO wt_test.race AS r (id, urn, note) VALUES ((args ->> 'id')::uuid, args ->> 'urn', args ->> 'note')
    ON CONFLICT (id) DO UPDATE SET note = EXCLUDED.note
    RETURNING (r.xmax = 0) INTO fresh;
    RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', jsonb_build_object('was_new', fresh));
END
$f$;
