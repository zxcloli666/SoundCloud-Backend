SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_on_repl_repair()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(NEW.tbl);
    r jsonb := NEW."row";
BEGIN
    IF NEW.src_node <> public.scd_node()
        AND ((public.scd_role() = 'core' AND cls IN ('M', 'A', 'K')) OR (public.scd_role() = 'api' AND cls IN ('C', 'M', 'A'))) THEN
        IF public.scd_role() = 'core' AND cls = 'M' THEN
            SELECT jsonb_object_agg(e.key, e.value) INTO r
            FROM jsonb_each(NEW."row") AS e
            WHERE e.key IN (
                SELECT unnest(l.cols)
                FROM public.scd_publication_lists() AS l
                WHERE l.node = 'api'
                  AND l.tbl = NEW.tbl
            );
        END IF;
        PERFORM public.scd_apply_row(NEW.tbl, r);
    END IF;
    RETURN NULL;
END
$f$;
