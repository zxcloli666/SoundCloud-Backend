SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_lock_group(rel text)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT CASE WHEN rel IN ('user_followings', 'user_likes_playlists', 'user_likes_tracks') THEN 'sync_queue' ELSE rel END
$f$;

CREATE OR REPLACE FUNCTION public.scd_drain_resend_counts(batch integer DEFAULT 500, OUT sent integer, OUT skipped integer, OUT gone integer)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    r record;
    body jsonb;
    rid uuid;
BEGIN
    sent := 0;
    skipped := 0;
    gone := 0;
    FOR r IN
        SELECT b.tbl, b.pk FROM (
            SELECT x.tbl, x.pk,
                   CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END AS live,
                   CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk -> 'pk' ELSE x.pk END AS live_pk
            FROM public.repl_resend x
            ORDER BY CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END,
                     CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk -> 'pk' ELSE x.pk END,
                     x.tbl, x.pk
            LIMIT batch
        ) b
        ORDER BY public.scd_lock_group(b.live), b.live = 'sync_queue', b.live, b.live_pk, b.tbl, b.pk
    LOOP
        IF r.tbl <> 'repl_tombstones' AND NOT public.scd_try_lock_row(r.tbl, r.pk) THEN
            skipped := skipped + 1;
            CONTINUE;
        END IF;
        PERFORM 1 FROM public.repl_resend x WHERE x.tbl = r.tbl AND x.pk = r.pk FOR UPDATE SKIP LOCKED;
        IF NOT FOUND THEN
            skipped := skipped + 1;
            CONTINUE;
        END IF;
        body := public.scd_row_json(r.tbl, r.pk);
        IF body IS NULL THEN
            gone := gone + 1;
        ELSE
            INSERT INTO public.repl_repair (id, src_node, tbl, pk, "row")
            VALUES (gen_random_uuid(), public.scd_node(), r.tbl, r.pk, body)
            RETURNING id INTO rid;
            DELETE FROM public.repl_repair x WHERE x.id = rid;
            sent := sent + 1;
        END IF;
        DELETE FROM public.repl_resend x WHERE x.tbl = r.tbl AND x.pk = r.pk;
    END LOOP;
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
        SELECT o.node, o.tbl, o.range, o.hash FROM (
            SELECT x.node, x.tbl, x.range, x.hash, x.at FROM public.repl_checksums x
            WHERE x.node <> 'core' AND x.tbl <> '*' AND x.range NOT LIKE 'wait %'
            ORDER BY x.at
            LIMIT answers
            FOR UPDATE SKIP LOCKED
        ) o
        ORDER BY public.scd_lock_group(o.tbl), o.tbl = 'sync_queue', o.tbl,
                 CASE WHEN pg_input_is_valid(o.hash, 'jsonb') THEN nullif(o.hash::jsonb -> 'lo', 'null') END NULLS FIRST,
                 o.at, o.node, o.range
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
