SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_verify_answer(probe uuid, rel text, k jsonb, op jsonb)
RETURNS boolean
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    c record;
    diff jsonb;
    held jsonb;
BEGIN
    IF EXISTS (SELECT 1 FROM public.repl_resend x WHERE (CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END) = rel) THEN
        IF current_setting('session_replication_role') = 'replica' THEN
            INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
            VALUES (public.scd_node(), rel, 'wait ' || probe::text, jsonb_build_object('key', k, 'op', op)::text, now())
            ON CONFLICT DO NOTHING;
        END IF;
        RETURN false;
    END IF;
    SELECT * INTO c FROM public.scd_verify_chunk(rel, nullif(k -> 'lo', 'null'), nullif(k -> 'hi', 'null'),
        ARRAY(SELECT jsonb_array_elements_text(op -> 'cols')));
    IF jsonb_typeof(op -> 'keys') = 'object' THEN
        diff := (SELECT jsonb_object_agg(coalesce(m.key, t.key), m.value)
                 FROM jsonb_each_text(c.keys) m
                 FULL JOIN jsonb_each_text(op -> 'keys') t ON t.key = m.key
                 WHERE m.value IS DISTINCT FROM t.value);
        IF diff IS NULL THEN
            RETURN true;
        END IF;
        IF public.scd_table_class(rel) = 'M' THEN
            held := (SELECT jsonb_object_agg(d.key, r.list)
                     FROM jsonb_object_keys(diff) d (key)
                     CROSS JOIN LATERAL (
                         SELECT jsonb_agg(jsonb_build_object('purged_v', x.purged_v, 'at', x.at) ORDER BY x.at) AS list
                         FROM public.repl_purges x WHERE x.tbl = rel AND x.pk = d.key::jsonb
                     ) r
                     WHERE r.list IS NOT NULL);
        END IF;
    ELSIF c.n = (op ->> 'n')::bigint AND c.h = op ->> 'h' THEN
        RETURN true;
    END IF;
    INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
    VALUES (public.scd_node(), rel, probe::text,
        jsonb_build_object('lo', k -> 'lo', 'hi', k -> 'hi', 'attempt', op -> 'attempt', 'n', c.n, 'keys', diff, 'receipts', held)::text, now())
    ON CONFLICT DO NOTHING;
    IF current_setting('session_replication_role') = 'replica' THEN
        INSERT INTO public.repl_resend (tbl, pk)
        VALUES ('repl_checksums', jsonb_build_object('node', public.scd_node(), 'tbl', rel, 'range', probe::text))
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_receipts(rel text, k jsonb, receipts jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF public.scd_role() <> 'api' OR public.scd_table_class(rel) IS DISTINCT FROM 'M' OR jsonb_typeof(receipts) IS DISTINCT FROM 'array' THEN
        RETURN;
    END IF;
    PERFORM public.scd_lock_row(rel, k);
    DELETE FROM public.repl_purges x
    WHERE x.tbl = rel AND x.pk = k
      AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(receipts) r
                      WHERE (r ->> 'purged_v')::bigint = x.purged_v AND (r ->> 'at')::timestamptz = x.at);
    INSERT INTO public.repl_purges (tbl, pk, purged_v, at)
    SELECT rel, k, (r ->> 'purged_v')::bigint, (r ->> 'at')::timestamptz FROM jsonb_array_elements(receipts) r
    ON CONFLICT DO NOTHING;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_repair_op(id uuid, rel text, k jsonb, op jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    CASE op ->> 'scd_op'
        WHEN 'probe' THEN
            PERFORM public.scd_verify_answer(id, rel, k, op);
        WHEN 'drop' THEN
            PERFORM public.scd_verify_drop(rel, k, op -> 'receipts');
        WHEN 'receipts' THEN
            PERFORM public.scd_verify_receipts(rel, k, op -> 'receipts');
        WHEN 'resend' THEN
            IF op ->> 'node' = public.scd_node() THEN
                INSERT INTO public.repl_resend (tbl, pk) VALUES (rel, k) ON CONFLICT DO NOTHING;
            END IF;
        ELSE
            NULL;
    END CASE;
EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'scd_repair_op % on public.% %: %', op ->> 'scd_op', rel, k, SQLERRM;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_settle(peer text, rel text, body jsonb, dry boolean)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(rel);
    tombs boolean := EXISTS (SELECT 1 FROM pg_catalog.pg_trigger g WHERE g.tgrelid = ('public.' || quote_ident(rel))::regclass AND g.tgname = 'zz_tomb');
    lo jsonb := nullif(body -> 'lo', 'null');
    hi jsonb := nullif(body -> 'hi', 'null');
    attempt integer := coalesce((body ->> 'attempt')::integer, 0);
    mine record;
    d record;
    k jsonb;
    image jsonb;
    receipts jsonb;
    held jsonb;
    skew boolean;
    tomb jsonb;
    acts text[];
    act text;
    report jsonb := '[]';
BEGIN
    SELECT * INTO mine FROM public.scd_verify_chunk(rel, lo, hi);
    IF jsonb_typeof(body -> 'keys') IS DISTINCT FROM 'object' THEN
        PERFORM public.scd_verify_send(rel, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'attempt', 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
        RETURN jsonb_build_array(jsonb_build_object('k', jsonb_build_object('lo', lo, 'hi', hi), 'action', 'recheck', 'attempt', 0));
    END IF;
    FOR d IN
        SELECT p.key, mine.keys ->> p.key AS ours, p.value AS theirs
        FROM jsonb_each_text(body -> 'keys') p
        WHERE (mine.keys ->> p.key) IS DISTINCT FROM p.value
        ORDER BY 1
    LOOP
        k := d.key::jsonb;
        receipts := NULL;
        skew := false;
        IF cls = 'M' THEN
            receipts := (SELECT jsonb_agg(jsonb_build_object('purged_v', x.purged_v, 'at', x.at) ORDER BY x.at)
                         FROM public.repl_purges x WHERE x.tbl = rel AND x.pk = k);
            held := CASE WHEN jsonb_typeof(body -> 'receipts' -> d.key) = 'array' THEN body -> 'receipts' -> d.key ELSE '[]' END;
            skew := EXISTS (
                SELECT 1 FROM (
                    SELECT (e ->> 'purged_v')::bigint AS v, (e ->> 'at')::timestamptz AS at, 'core' AS side
                    FROM jsonb_array_elements(coalesce(receipts, '[]')) e
                    UNION ALL
                    SELECT (e ->> 'purged_v')::bigint, (e ->> 'at')::timestamptz, 'peer'
                    FROM jsonb_array_elements(held) e
                ) s
                GROUP BY s.v, s.at
                HAVING count(DISTINCT s.side) = 1);
        END IF;
        IF d.ours IS NOT NULL THEN
            acts := CASE WHEN cls <> 'C' AND d.theirs IS NOT NULL THEN ARRAY['row', 'resend'] ELSE ARRAY['row'] END;
            IF skew THEN
                acts := ARRAY['receipts'] || acts;
            END IF;
        ELSIF cls = 'C' OR (cls = 'A' AND NOT tombs) THEN
            acts := ARRAY['drop'];
        ELSIF cls = 'M' THEN
            acts := CASE WHEN receipts IS NULL THEN ARRAY['resend'] ELSE ARRAY['drop'] END;
        ELSE
            tomb := (SELECT jsonb_build_object('tbl', x.tbl, 'pk', x.pk, 'v', x.v)
                     FROM public.repl_tombstones x WHERE x.tbl = rel AND x.pk = k ORDER BY x.v DESC LIMIT 1);
            acts := CASE WHEN tomb IS NULL THEN ARRAY['resend'] ELSE ARRAY['tomb'] END;
        END IF;
        FOREACH act IN ARRAY acts LOOP
            IF NOT dry THEN
                CASE act
                    WHEN 'receipts' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'receipts', 'receipts', coalesce(receipts, '[]')));
                    WHEN 'row' THEN
                        image := public.scd_row_json(rel, k);
                        IF image IS NOT NULL THEN
                            PERFORM public.scd_repair_send(rel, k, image);
                        END IF;
                    WHEN 'resend' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'resend', 'node', peer));
                    WHEN 'drop' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'drop', 'receipts', receipts));
                    WHEN 'tomb' THEN
                        PERFORM public.scd_repair_send('repl_tombstones', tomb, public.scd_row_json('repl_tombstones', tomb));
                END CASE;
            END IF;
            report := report || jsonb_build_array(jsonb_build_object('k', k, 'action', act, 'attempt', attempt));
        END LOOP;
    END LOOP;
    IF jsonb_array_length(report) > 0 AND NOT dry AND attempt < 3 THEN
        PERFORM public.scd_verify_send(rel, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'attempt', attempt + 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
    END IF;
    RETURN report;
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
    IF tbl NOT IN ('cmd_outbox', 'oauth_app_tokens_local', 'repl_purges') THEN
        RAISE EXCEPTION 'scd node retention: unknown table %', tbl USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF tbl = 'repl_purges' AND (public.scd_mode() <> 'split' OR public.scd_role() <> 'api') THEN
        RETURN;
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
    IF tbl = 'repl_purges' THEN
        WITH d AS (
            DELETE FROM public.repl_purges t WHERE t.ctid IN (
                SELECT x.ctid FROM public.repl_purges x
                WHERE x.at < now() - greatest(keep, public.scd_dedup_window()) ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
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
