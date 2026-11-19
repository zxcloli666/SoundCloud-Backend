DO $$
BEGIN
    IF public.scd_role() IN ('core', 'api') THEN
        DELETE FROM public.repl_resend
        WHERE tbl = 'catalog_audience'
           OR (tbl = 'repl_tombstones' AND pk ->> 'tbl' = 'catalog_audience');
        DELETE FROM public.repl_checksums
        WHERE tbl = 'catalog_audience';
        DELETE FROM public.cmd_pending
        WHERE tbl = 'catalog_audience';
    END IF;
    IF public.scd_role() = 'core' THEN
        DELETE FROM public.repl_tombstones
        WHERE tbl = 'catalog_audience';
    END IF;
END
$$;

DO $$
DECLARE
    tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.catalog_audience IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.catalog_audience IN ACCESS EXCLUSIVE MODE;
            EXIT;
        EXCEPTION WHEN lock_not_available THEN
            tries := tries + 1;
            IF tries = 120 THEN
                RAISE;
            END IF;
            PERFORM pg_catalog.pg_sleep(1);
        END;
    END LOOP;
END
$$;

DROP TABLE public.catalog_audience;

DROP FUNCTION IF EXISTS public.scd_owner_catalog_audience();
DROP FUNCTION IF EXISTS public.scd_purge_catalog_audience(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_hash_catalog_audience(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_json_catalog_audience(jsonb);
DROP FUNCTION IF EXISTS public.scd_owner_copy_catalog_audience();
DROP SEQUENCE IF EXISTS public.scd_guard_catalog_audience;
