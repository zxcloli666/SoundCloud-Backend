SET LOCAL lock_timeout = '5s';

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
        SELECT x.tbl, x.pk FROM public.repl_resend x
        ORDER BY CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END,
                 CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk -> 'pk' ELSE x.pk END,
                 x.tbl, x.pk
        LIMIT batch
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

CREATE OR REPLACE FUNCTION public.scd_drain_resend(batch integer DEFAULT 500)
RETURNS integer
LANGUAGE sql
SET search_path = public, pg_catalog
AS $f$
SELECT c.sent + c.skipped + c.gone FROM public.scd_drain_resend_counts(batch) c
$f$;

CREATE OR REPLACE FUNCTION public.scd_sweep(batch integer DEFAULT 500, OUT state text, OUT sent integer, OUT skipped integer, OUT gone integer, OUT queued bigint)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
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
    END IF;
    queued := (SELECT count(*) FROM (SELECT 1 FROM public.repl_resend LIMIT 100000) x);
END
$f$;
