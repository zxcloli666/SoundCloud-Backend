SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_dedup_window()
RETURNS interval
LANGUAGE sql
IMMUTABLE
SET search_path = public, pg_catalog
AS $f$
SELECT interval '7 days'
$f$;

CREATE OR REPLACE FUNCTION public.scd_redact_args(kind text, args jsonb)
RETURNS jsonb
LANGUAGE sql
IMMUTABLE
SET search_path = public, pg_catalog
AS $f$
SELECT CASE
    WHEN kind = 'connection.login' AND args ?| ARRAY['access_token', 'refresh_token']
        THEN (args - ARRAY['access_token', 'refresh_token']) || jsonb_build_object('redacted', true)
    ELSE args
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_login_replayable(args jsonb)
RETURNS boolean
LANGUAGE plpgsql
STABLE
SET search_path = public, pg_catalog
AS $f$
BEGIN
    RETURN EXISTS (
        SELECT 1 FROM public.soundcloud_connections c
        WHERE c.id = (args ->> 'connection_id')::uuid AND c.login_v < (args ->> 'login_v')::bigint
    );
EXCEPTION WHEN invalid_text_representation OR numeric_value_out_of_range THEN
    RETURN false;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_retention_reason(held text)
RETURNS text
LANGUAGE sql
IMMUTABLE
SET search_path = public, pg_catalog
AS $f$
SELECT CASE
    WHEN held IS NULL THEN NULL
    WHEN held LIKE '%in recovery%' THEN 'recovery'
    WHEN held LIKE '%runs on core only%' THEN 'role'
    WHEN held ~ '(subscription|apply worker|apply is|message from core)' THEN 'subscription'
    WHEN held LIKE '%slot%' THEN 'slot'
    WHEN held LIKE '%resend%' THEN 'resend'
    WHEN held LIKE '%wait for core%' THEN 'pending'
    ELSE 'other'
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_connection_login(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    connection_id uuid := (args ->> 'connection_id')::uuid;
    v bigint := (args ->> 'login_v')::bigint;
    stamp timestamptz := (args ->> 'at')::timestamptz;
    installed boolean;
BEGIN
    IF args ->> 'access_token' IS NULL OR args ->> 'refresh_token' IS NULL THEN
        RAISE EXCEPTION 'connection.login of %: the token pair is missing or was redacted', connection_id
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    UPDATE public.soundcloud_connections
    SET soundcloud_user_id = args ->> 'soundcloud_user_id',
        username = coalesce(nullif(args ->> 'username', ''), username),
        oauth_app_id = (args ->> 'oauth_app_id')::uuid,
        access_token = args ->> 'access_token',
        refresh_token = args ->> 'refresh_token',
        expires_at = (args ->> 'expires_at')::timestamptz,
        scope = args ->> 'scope',
        token_v = greatest(token_v, v),
        login_v = v,
        refresh_generation = refresh_generation + 1,
        refresh_failure_count = 0,
        refresh_rejection_count = 0,
        first_refresh_rejection_at = NULL,
        refresh_lease_id = NULL,
        refresh_lease_expires_at = NULL,
        last_refresh_success_at = stamp,
        last_refresh_error_kind = NULL,
        last_refresh_error = NULL,
        retry_at = NULL,
        updated_at = stamp
    WHERE id = connection_id AND login_v < v;
    installed := FOUND;
    IF NOT installed THEN
        UPDATE public.soundcloud_connections
        SET updated_at = greatest(updated_at, stamp)
        WHERE id = connection_id;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
        END IF;
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'soundcloud_connections', 'pk', jsonb_build_object('id', connection_id))),
        'output', installed
    );
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_dispatch()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET session_replication_role = origin
SET lock_timeout = '5s'
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
AS $f$
DECLARE
    out jsonb;
    k jsonb;
    h text;
    st text;
    msg text;
    snap pg_snapshot;
    nsp text;
    rel text;
    con text;
    det text;
    acked timestamptz;
BEGIN
    DELETE FROM public.cmd_done d WHERE d.node = NEW.node AND d.id = NEW.id RETURNING d.status, d.at INTO st, acked;
    IF FOUND THEN
        INSERT INTO public.cmd_done (node, id, status, at) VALUES (NEW.node, NEW.id, st, acked);
        RETURN NULL;
    END IF;
    INSERT INTO public.cmd_done (node, id, status) VALUES (NEW.node, NEW.id, 'ok');
    PERFORM set_config('scd.cmd', NEW.id::text, true);
    snap := pg_current_snapshot();
    BEGIN
        PERFORM 1 FROM public.scd_kinds() x WHERE x.kind = NEW.kind;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'scd dispatch: unknown kind %', NEW.kind USING ERRCODE = 'invalid_parameter_value';
        END IF;
        IF NEW.created_at < now() - public.scd_dedup_window() + interval '1 hour' THEN
            RAISE EXCEPTION 'scd dispatch: command % of % is older than the dedup window, replay it only if core never ran it', NEW.id, NEW.node
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(NEW.kind)) INTO out USING NEW.args;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT,
            nsp = SCHEMA_NAME, rel = TABLE_NAME, con = CONSTRAINT_NAME, det = PG_EXCEPTION_DETAIL;
        IF public.scd_transient(st) THEN
            RAISE;
        END IF;
        IF st = '23505' AND public.scd_unique_race(nsp, rel, con, det, snap) IS NOT NULL THEN
            RAISE;
        END IF;
        INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message)
        VALUES (NEW.node, NEW.id, NEW.kind, NEW.args, st, msg)
        ON CONFLICT DO NOTHING;
        UPDATE public.cmd_done d SET status = 'dead' WHERE d.node = NEW.node AND d.id = NEW.id;
    END;
    PERFORM set_config('scd.cmd', '', true);
    BEGIN
        FOR k IN SELECT e.value FROM jsonb_array_elements(coalesce(NEW.touched, '[]')) AS e LOOP
            CONTINUE WHEN public.scd_table_class(k ->> 'tbl') IS DISTINCT FROM 'C';
            h := public.scd_row_hash(k ->> 'tbl', k -> 'pk');
            IF h IS NULL AND k ->> 'hash' IS NOT NULL THEN
                INSERT INTO public.cmd_purge (node, id, tbl, pk)
                VALUES (NEW.node, NEW.id, k ->> 'tbl', k -> 'pk')
                ON CONFLICT DO NOTHING;
            ELSIF h IS NOT NULL AND h IS DISTINCT FROM k ->> 'hash' THEN
                INSERT INTO public.cmd_reassert (node, id, tbl, pk, "row")
                VALUES (NEW.node, NEW.id, k ->> 'tbl', k -> 'pk', public.scd_row_json(k ->> 'tbl', k -> 'pk'))
                ON CONFLICT DO NOTHING;
            END IF;
        END LOOP;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT;
        IF public.scd_transient(st) THEN
            RAISE;
        END IF;
        INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message)
        VALUES (NEW.node, NEW.id, NEW.kind, NEW.args, st, 'touched: ' || msg)
        ON CONFLICT DO NOTHING;
        UPDATE public.cmd_done d SET status = 'dead' WHERE d.node = NEW.node AND d.id = NEW.id;
    END;
    RETURN NULL;
END
$f$;

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
SET search_path = public, pg_catalog
SET lock_timeout = '2s'
AS $f$
DECLARE
    cutoff timestamptz := now() - greatest(keep, public.scd_dedup_window());
BEGIN
    deleted := 0;
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd retention runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    IF tbl NOT IN ('cmd_reassert', 'cmd_purge', 'cmd_done', 'cmd_dead', 'repl_purges', 'repl_tombstones', 'storage_event_repairs') THEN
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

CREATE OR REPLACE FUNCTION public.scd_dead_redact(batch integer DEFAULT 1000)
RETURNS bigint
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET lock_timeout = '2s'
AS $f$
DECLARE
    n bigint;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd retention runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    WITH r AS (
        UPDATE public.cmd_dead t SET args = public.scd_redact_args(t.kind, t.args)
        WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_dead x
            WHERE x.kind = 'connection.login' AND x.args ?| ARRAY['access_token', 'refresh_token']
              AND NOT public.scd_login_replayable(x.args)
            ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO n FROM r;
    RETURN n;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_node_retention_guard(lag_bytes bigint DEFAULT 67108864, stale interval DEFAULT interval '3 minutes')
RETURNS TABLE (peer text, reason text)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    me oid := (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = current_database());
BEGIN
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'api' THEN
        RETURN;
    END IF;
    IF pg_is_in_recovery() THEN
        RETURN QUERY SELECT public.scd_node(), 'node is in recovery';
        RETURN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_subscription s WHERE s.subdbid = me) THEN
        RETURN QUERY SELECT 'subscriptions'::text, 'no subscription from core';
    END IF;
    RETURN QUERY
    SELECT s.subname::text,
           CASE
               WHEN NOT s.subenabled THEN 'subscription disabled'
               WHEN w.pid IS NULL THEN 'no apply worker'
               WHEN w.last_msg_receipt_time IS NULL THEN 'no message from core yet'
               WHEN w.last_msg_receipt_time < now() - stale THEN format('no message from core for %s', date_trunc('second', now() - w.last_msg_receipt_time))
               ELSE format('apply is %s bytes behind', pg_wal_lsn_diff(w.received_lsn, w.latest_end_lsn)::bigint)
           END
    FROM pg_catalog.pg_subscription s
    LEFT JOIN pg_catalog.pg_stat_subscription w ON w.subid = s.oid AND w.relid IS NULL AND w.leader_pid IS NULL
    WHERE s.subdbid = me
      AND (NOT s.subenabled OR w.pid IS NULL OR w.last_msg_receipt_time IS NULL OR w.last_msg_receipt_time < now() - stale
           OR pg_wal_lsn_diff(w.received_lsn, w.latest_end_lsn) > lag_bytes);
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_replication_slots sl WHERE sl.slot_type = 'logical' AND sl.plugin = 'pgoutput' AND sl.database = current_database()) THEN
        RETURN QUERY SELECT 'slots'::text, 'no slot for core';
    END IF;
    RETURN QUERY
    SELECT sl.slot_name::text,
           CASE
               WHEN NOT sl.active THEN 'slot inactive'
               WHEN sl.invalidation_reason IS NOT NULL OR sl.wal_status IN ('lost', 'unreserved') THEN 'slot wal status ' || coalesce(sl.invalidation_reason, sl.wal_status)
               WHEN sl.confirmed_flush_lsn IS NULL THEN 'slot has no confirmed position'
               ELSE format('slot is %s bytes behind', pg_wal_lsn_diff(pg_current_wal_lsn(), sl.confirmed_flush_lsn)::bigint)
           END
    FROM pg_catalog.pg_replication_slots sl
    WHERE sl.slot_type = 'logical' AND sl.plugin = 'pgoutput' AND sl.database = current_database()
      AND (NOT sl.active OR sl.invalidation_reason IS NOT NULL OR sl.wal_status IN ('lost', 'unreserved')
           OR sl.confirmed_flush_lsn IS NULL OR pg_wal_lsn_diff(pg_current_wal_lsn(), sl.confirmed_flush_lsn) > lag_bytes);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_node_retention_batch(
    tbl text,
    keep interval DEFAULT interval '2 days',
    batch integer DEFAULT 1000,
    lag_bytes bigint DEFAULT 67108864,
    stale interval DEFAULT interval '3 minutes',
    OUT deleted bigint,
    OUT redacted bigint,
    OUT held text
)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET lock_timeout = '2s'
AS $f$
DECLARE
    done_keep interval := least(greatest(keep, interval '1 day'), public.scd_dedup_window() - interval '1 day');
    secret_cutoff timestamptz := now() - interval '1 hour';
    stuck bigint;
BEGIN
    deleted := 0;
    redacted := 0;
    IF tbl NOT IN ('cmd_outbox', 'oauth_app_tokens_local') THEN
        RAISE EXCEPTION 'scd node retention: unknown table %', tbl USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.retention.node', 0)) THEN
        RETURN;
    END IF;
    IF tbl = 'oauth_app_tokens_local' THEN
        WITH d AS (
            DELETE FROM public.oauth_app_tokens_local t WHERE t.ctid IN (
                SELECT x.ctid FROM public.oauth_app_tokens_local x
                WHERE x.expires_at < secret_cutoff ORDER BY x.expires_at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
        RETURN;
    END IF;
    SELECT string_agg(g.peer || ': ' || g.reason, '; ' ORDER BY g.peer, g.reason) INTO held FROM public.scd_node_retention_guard(lag_bytes, stale) g;
    IF held IS NOT NULL THEN
        RETURN;
    END IF;
    WITH r AS (
        UPDATE public.cmd_outbox t SET args = public.scd_redact_args(t.kind, t.args)
        WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_outbox x
            WHERE x.kind = 'connection.login' AND x.args ?| ARRAY['access_token', 'refresh_token']
              AND (x.done_at < secret_cutoff OR (x.done_at IS NULL AND x.created_at < now() - public.scd_dedup_window()))
            ORDER BY x.created_at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO redacted FROM r;
    WITH d AS (
        DELETE FROM public.cmd_outbox t WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_outbox x
            WHERE x.created_at < now() - done_keep AND x.done_at < now() - done_keep
            ORDER BY x.created_at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO deleted FROM d;
    SELECT count(*) INTO stuck FROM (
        SELECT 1 FROM public.cmd_outbox x WHERE x.created_at < now() - done_keep AND x.done_at IS NULL LIMIT 100000
    ) s;
    IF stuck > 0 THEN
        held := format('%s commands wait for core longer than %s', stuck, done_keep);
    END IF;
END
$f$;
