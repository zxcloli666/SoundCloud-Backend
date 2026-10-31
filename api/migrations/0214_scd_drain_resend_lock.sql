SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_try_lock_row(tbl text, k jsonb)
RETURNS boolean
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cond text;
    got bigint;
BEGIN
    SELECT string_agg(format('t.%I = r.%I', a.attname, a.attname), ' AND ' ORDER BY a.attnum) INTO cond
    FROM pg_index i
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
    WHERE i.indrelid = ('public.' || quote_ident(tbl))::regclass AND i.indisprimary;
    IF cond IS NULL THEN
        RETURN true;
    END IF;
    EXECUTE format('SELECT count(*) FROM (SELECT 1 FROM public.%I t, jsonb_populate_record(NULL::public.%I, $1) r WHERE %s FOR UPDATE OF t SKIP LOCKED) s',
        tbl, tbl, cond) INTO got USING k;
    IF got > 0 THEN
        RETURN true;
    END IF;
    EXECUTE format('SELECT count(*) FROM public.%I t, jsonb_populate_record(NULL::public.%I, $1) r WHERE %s', tbl, tbl, cond)
        INTO got USING k;
    RETURN got = 0;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_drain_resend(batch integer DEFAULT 500)
RETURNS integer
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    r record;
    body jsonb;
    rid uuid;
    n integer := 0;
BEGIN
    FOR r IN
        SELECT x.tbl, x.pk FROM public.repl_resend x
        ORDER BY CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END,
                 CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk -> 'pk' ELSE x.pk END,
                 x.tbl, x.pk
        LIMIT batch
    LOOP
        n := n + 1;
        IF r.tbl <> 'repl_tombstones' AND NOT public.scd_try_lock_row(r.tbl, r.pk) THEN
            CONTINUE;
        END IF;
        PERFORM 1 FROM public.repl_resend x WHERE x.tbl = r.tbl AND x.pk = r.pk FOR UPDATE SKIP LOCKED;
        IF NOT FOUND THEN
            CONTINUE;
        END IF;
        body := public.scd_row_json(r.tbl, r.pk);
        IF body IS NOT NULL THEN
            INSERT INTO public.repl_repair (id, src_node, tbl, pk, "row")
            VALUES (gen_random_uuid(), public.scd_node(), r.tbl, r.pk, body)
            RETURNING id INTO rid;
            DELETE FROM public.repl_repair x WHERE x.id = rid;
        END IF;
        DELETE FROM public.repl_resend x WHERE x.tbl = r.tbl AND x.pk = r.pk;
    END LOOP;
    RETURN n;
END
$f$;
