SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_lock_row(tbl text, k jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cond text;
BEGIN
    SELECT string_agg(format('t.%I = r.%I', a.attname, a.attname), ' AND ' ORDER BY a.attnum) INTO cond
    FROM pg_index i
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
    WHERE i.indrelid = ('public.' || quote_ident(tbl))::regclass AND i.indisprimary;
    IF cond IS NULL THEN
        RETURN;
    END IF;
    EXECUTE format('SELECT 1 FROM public.%I t, jsonb_populate_record(NULL::public.%I, $1) r WHERE %s FOR UPDATE OF t', tbl, tbl, cond)
        USING k;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_on_cmd_purge()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NEW.node = public.scd_node() AND public.scd_table_class(NEW.tbl) = 'C' THEN
        PERFORM public.scd_lock_row(NEW.tbl, NEW.pk);
        IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = NEW.tbl AND p.pk = NEW.pk) THEN
            PERFORM public.scd_purge(NEW.tbl, NEW.pk);
        END IF;
    END IF;
    RETURN NULL;
END
$f$;
