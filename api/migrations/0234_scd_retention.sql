SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_resend_report()
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    n text := (SELECT count(*) FROM (SELECT 1 FROM public.repl_resend LIMIT 100000) x)::text;
    last record;
BEGIN
    SELECT c.hash, c.at INTO last FROM public.repl_checksums c
    WHERE c.node = public.scd_node() AND c.tbl = '*' AND c.range LIKE 'resend %'
    ORDER BY c.at DESC LIMIT 1;
    IF NOT FOUND OR last.hash IS DISTINCT FROM n OR last.at < now() - interval '60 seconds' THEN
        INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
        VALUES (public.scd_node(), '*', 'resend ' || floor(extract(epoch FROM clock_timestamp()) * 1000000)::bigint, n, now());
    END IF;
    DELETE FROM public.repl_checksums c
    WHERE c.node = public.scd_node() AND c.tbl = '*' AND c.range LIKE 'resend %' AND c.at < now() - interval '10 minutes';
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_retention_guard(lag_bytes bigint DEFAULT 67108864, stale interval DEFAULT interval '3 minutes')
RETURNS TABLE (peer text, reason text)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    peers text[] := ARRAY['main', 'star'];
    p text;
    rep record;
    me oid := (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = current_database());
BEGIN
    IF public.scd_role() <> 'core' THEN
        RETURN QUERY SELECT public.scd_node(), 'retention runs on core only';
        RETURN;
    END IF;
    IF public.scd_mode() <> 'split' THEN
        RETURN;
    END IF;
    IF pg_is_in_recovery() THEN
        RETURN QUERY SELECT public.scd_node(), 'node is in recovery';
        RETURN;
    END IF;
    IF (SELECT count(*) FROM pg_catalog.pg_subscription s WHERE s.subdbid = me) < cardinality(peers) THEN
        RETURN QUERY SELECT 'subscriptions'::text, format('%s of %s present', (SELECT count(*) FROM pg_catalog.pg_subscription s WHERE s.subdbid = me), cardinality(peers));
    END IF;
    RETURN QUERY
    SELECT s.subname::text,
           CASE
               WHEN NOT s.subenabled THEN 'subscription disabled'
               WHEN w.pid IS NULL THEN 'no apply worker'
               ELSE format('apply is %s bytes behind', pg_wal_lsn_diff(w.received_lsn, w.latest_end_lsn)::bigint)
           END
    FROM pg_catalog.pg_subscription s
    LEFT JOIN pg_catalog.pg_stat_subscription w ON w.subid = s.oid AND w.relid IS NULL AND w.leader_pid IS NULL
    WHERE s.subdbid = me
      AND (NOT s.subenabled OR w.pid IS NULL OR pg_wal_lsn_diff(w.received_lsn, w.latest_end_lsn) > lag_bytes);
    IF (SELECT count(*) FROM pg_catalog.pg_replication_slots sl WHERE sl.slot_type = 'logical' AND sl.plugin = 'pgoutput' AND sl.database = current_database()) < cardinality(peers) THEN
        RETURN QUERY SELECT 'slots'::text, format('%s of %s present', (SELECT count(*) FROM pg_catalog.pg_replication_slots sl WHERE sl.slot_type = 'logical' AND sl.plugin = 'pgoutput' AND sl.database = current_database()), cardinality(peers));
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
    FOREACH p IN ARRAY peers LOOP
        SELECT c.hash, c.at INTO rep FROM public.repl_checksums c
        WHERE c.node = p AND c.tbl = '*' AND c.range LIKE 'resend %'
        ORDER BY c.at DESC LIMIT 1;
        IF NOT FOUND THEN
            RETURN QUERY SELECT p, 'no resend report';
        ELSIF rep.at < now() - stale THEN
            RETURN QUERY SELECT p, format('resend report is %s old', date_trunc('second', now() - rep.at));
        ELSIF rep.hash !~ '^[0-9]+$' THEN
            RETURN QUERY SELECT p, 'resend report is unreadable';
        ELSIF rep.hash <> '0' THEN
            RETURN QUERY SELECT p, format('%s keys wait for resend', rep.hash);
        END IF;
    END LOOP;
    IF EXISTS (SELECT 1 FROM public.repl_resend) THEN
        RETURN QUERY SELECT public.scd_node(), 'keys wait for resend';
    END IF;
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
    cutoff timestamptz := now() - greatest(keep, interval '7 days');
BEGIN
    deleted := 0;
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd retention runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    IF tbl NOT IN ('cmd_done', 'cmd_dead', 'repl_purges', 'repl_tombstones') THEN
        RAISE EXCEPTION 'scd retention: unknown table %', tbl USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF tbl <> 'cmd_dead' THEN
        SELECT string_agg(g.peer || ': ' || g.reason, '; ' ORDER BY g.peer, g.reason) INTO held FROM public.scd_retention_guard(lag_bytes, stale) g;
        IF held IS NOT NULL THEN
            RETURN;
        END IF;
    END IF;
    IF tbl = 'cmd_done' THEN
        WITH d AS (
            DELETE FROM public.cmd_done t WHERE t.ctid IN (
                SELECT x.ctid FROM public.cmd_done x WHERE x.at < cutoff ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
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
    ELSE
        WITH d AS (
            DELETE FROM public.repl_tombstones t WHERE t.ctid IN (
                SELECT x.ctid FROM public.repl_tombstones x WHERE x.at < cutoff ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_replay_dead(p_node text, p_id uuid)
RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET lock_timeout = '5s'
AS $f$
DECLARE
    d public.cmd_dead%ROWTYPE;
    st text;
    msg text;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd replay runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    SELECT * INTO d FROM public.cmd_dead x WHERE x.node = p_node AND x.id = p_id FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN
        RETURN 'missing';
    END IF;
    PERFORM set_config('scd.cmd', d.id::text, true);
    BEGIN
        PERFORM 1 FROM public.scd_kinds() k WHERE k.kind = d.kind;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'scd replay: unknown kind %', d.kind USING ERRCODE = 'invalid_parameter_value';
        END IF;
        EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(d.kind)) USING d.args;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT;
        PERFORM set_config('scd.cmd', '', true);
        IF public.scd_transient(st) THEN
            RAISE;
        END IF;
        UPDATE public.cmd_dead x SET sqlstate = st, message = msg WHERE x.node = p_node AND x.id = p_id;
        RETURN 'failed ' || st || ': ' || msg;
    END;
    PERFORM set_config('scd.cmd', '', true);
    DELETE FROM public.cmd_dead x WHERE x.node = p_node AND x.id = p_id;
    RETURN 'ok';
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_sweep(batch integer DEFAULT 500, OUT state text, OUT sent integer, OUT skipped integer, OUT gone integer, OUT queued bigint)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    w record;
BEGIN
    sent := 0;
    skipped := 0;
    gone := 0;
    queued := 0;
    IF public.scd_mode() <> 'split' THEN
        state := 'legacy';
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.sweep', 0)) THEN
        state := 'busy';
        RETURN;
    END IF;
    state := 'ran';
    SELECT c.sent, c.skipped, c.gone INTO sent, skipped, gone FROM public.scd_drain_resend_counts(batch) c;
    IF public.scd_role() = 'api' THEN
        DELETE FROM public.repl_checksums x WHERE x.node = public.scd_node() AND x.at < now() - interval '1 day';
        FOR w IN
            SELECT x.tbl, x.range, x.hash::jsonb AS body FROM public.repl_checksums x
            WHERE x.node = public.scd_node() AND x.range LIKE 'wait %'
              AND NOT EXISTS (SELECT 1 FROM public.repl_resend r WHERE (CASE WHEN r.tbl = 'repl_tombstones' THEN r.pk ->> 'tbl' ELSE r.tbl END) = x.tbl)
            ORDER BY x.at
            LIMIT 10
        LOOP
            IF public.scd_verify_answer(substr(w.range, 6)::uuid, w.tbl, w.body -> 'key', w.body -> 'op') THEN
                DELETE FROM public.repl_checksums x WHERE x.node = public.scd_node() AND x.tbl = w.tbl AND x.range = w.range;
            END IF;
        END LOOP;
        PERFORM public.scd_resend_report();
    END IF;
    queued := (SELECT count(*) FROM (SELECT 1 FROM public.repl_resend LIMIT 100000) x);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_repair(dry boolean DEFAULT false, answers integer DEFAULT 50)
RETURNS TABLE (peer text, rel text, k jsonb, action text, attempt integer)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    a record;
    body jsonb;
    report jsonb;
    r jsonb;
BEGIN
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.verify', 0)) THEN
        RETURN;
    END IF;
    FOR a IN
        SELECT x.node, x.tbl, x.range, x.hash FROM public.repl_checksums x
        WHERE x.node <> 'core' AND x.tbl <> '*' AND x.range NOT LIKE 'wait %'
        ORDER BY x.at
        LIMIT answers
        FOR UPDATE SKIP LOCKED
    LOOP
        BEGIN
            body := a.hash::jsonb;
            report := public.scd_verify_settle(a.node, a.tbl, body, dry);
        EXCEPTION WHEN OTHERS THEN
            report := jsonb_build_array(jsonb_build_object('k', NULL, 'action', 'error: ' || SQLERRM, 'attempt', 0));
        END;
        DELETE FROM public.repl_checksums x WHERE x.node = a.node AND x.tbl = a.tbl AND x.range = a.range;
        FOR r IN SELECT e FROM jsonb_array_elements(report) e LOOP
            peer := a.node;
            rel := a.tbl;
            k := r -> 'k';
            action := r ->> 'action';
            attempt := (r ->> 'attempt')::integer;
            RETURN NEXT;
        END LOOP;
    END LOOP;
END
$f$;
