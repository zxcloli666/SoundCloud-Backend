SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_retention_batch(
    tbl text,
    keep interval DEFAULT interval '7 days',
    batch integer DEFAULT 1000,
    lag_bytes bigint DEFAULT 67108864,
    stale interval DEFAULT interval '3 minutes',
    OUT deleted bigint,
    OUT held text
)
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
SET lock_timeout = '2s'
AS $f$
DECLARE
    cutoff timestamptz := now() - CASE WHEN tbl = 'track_seeks' THEN interval '180 days' ELSE greatest(keep, public.scd_dedup_window()) END;
BEGIN
    deleted := 0;
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd retention runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    IF tbl NOT IN ('cmd_reassert', 'cmd_purge', 'cmd_done', 'cmd_dead', 'repl_purges', 'repl_tombstones', 'storage_event_repairs', 'track_seeks') THEN
        RAISE EXCEPTION 'scd retention: unknown table %', tbl USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF tbl NOT IN ('cmd_dead', 'storage_event_repairs') THEN
        SELECT string_agg(g.peer || ': ' || g.reason, '; ' ORDER BY g.peer, g.reason) INTO held FROM public.scd_retention_guard(lag_bytes, stale) g;
        IF held IS NOT NULL THEN
            RETURN;
        END IF;
    END IF;
    IF tbl = 'cmd_reassert' THEN
        WITH d AS (
            DELETE FROM public.cmd_reassert t WHERE t.ctid IN (
                SELECT x.ctid FROM public.cmd_done p JOIN public.cmd_reassert x ON x.node = p.node AND x.id = p.id
                WHERE p.at < cutoff ORDER BY p.at LIMIT batch FOR UPDATE OF x SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'cmd_purge' THEN
        WITH d AS (
            DELETE FROM public.cmd_purge t WHERE t.ctid IN (
                SELECT x.ctid FROM public.cmd_done p JOIN public.cmd_purge x ON x.node = p.node AND x.id = p.id
                WHERE p.at < cutoff ORDER BY p.at LIMIT batch FOR UPDATE OF x SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'cmd_done' THEN
        WITH d AS (
            DELETE FROM public.cmd_done t WHERE t.ctid IN (
                SELECT x.ctid FROM public.cmd_done x
                WHERE x.at < cutoff
                  AND NOT EXISTS (SELECT 1 FROM public.cmd_reassert r WHERE r.node = x.node AND r.id = x.id)
                  AND NOT EXISTS (SELECT 1 FROM public.cmd_purge r WHERE r.node = x.node AND r.id = x.id)
                ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'cmd_dead' THEN
        WITH d AS (
            DELETE FROM public.cmd_dead t WHERE t.ctid IN (
                SELECT x.ctid FROM public.cmd_dead x WHERE x.at < cutoff ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'repl_purges' THEN
        WITH d AS (
            DELETE FROM public.repl_purges t WHERE t.ctid IN (
                SELECT x.ctid FROM public.repl_purges x WHERE x.at < cutoff ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'repl_tombstones' THEN
        WITH d AS (
            DELETE FROM public.repl_tombstones t WHERE t.ctid IN (
                SELECT x.ctid FROM public.repl_tombstones x WHERE x.at < cutoff ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSIF tbl = 'track_seeks' THEN
        WITH d AS (
            DELETE FROM public.track_seeks t WHERE t.ctid IN (
                SELECT x.ctid FROM public.track_seeks x
                WHERE x.id < (lpad(to_hex(floor(extract(epoch FROM cutoff) * 1000)::bigint), 12, '0') || repeat('0', 20))::uuid AND x.created_at < cutoff
                ORDER BY x.id LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    ELSE
        WITH d AS (
            DELETE FROM public.storage_event_repairs t WHERE t.ctid IN (
                SELECT x.ctid FROM public.storage_event_repairs x
                WHERE x.uploaded_at < (cutoff AT TIME ZONE 'UTC') ORDER BY x.uploaded_at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    END IF;
END
$f$;
