DO $$
BEGIN
    IF public.scd_role() IN ('core', 'api') THEN
        DELETE FROM public.repl_resend
        WHERE tbl IN ('remote_grants', 'remote_link_codes')
           OR (tbl = 'repl_tombstones' AND pk ->> 'tbl' IN ('remote_grants', 'remote_link_codes'));
        DELETE FROM public.repl_checksums
        WHERE tbl IN ('remote_grants', 'remote_link_codes');
    END IF;
    IF public.scd_role() = 'core' THEN
        DELETE FROM public.repl_tombstones
        WHERE tbl IN ('remote_grants', 'remote_link_codes');
        DELETE FROM public.background_jobs
        WHERE kind = 'auth.cleanup_remote_links';
        DELETE FROM public.background_schedules
        WHERE kind = 'auth.cleanup_remote_links';
        DELETE FROM public.background_job_failures
        WHERE kind = 'auth.cleanup_remote_links';
    END IF;
END
$$;

DO $$
DECLARE
    tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.remote_grants, public.remote_link_codes IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.remote_grants, public.remote_link_codes IN ACCESS EXCLUSIVE MODE;
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

DROP TABLE public.remote_grants;

DROP TABLE public.remote_link_codes;

DROP FUNCTION IF EXISTS public.scd_merge_remote_grants();
DROP FUNCTION IF EXISTS public.scd_tomb_remote_grants();
DROP FUNCTION IF EXISTS public.scd_tomb_check_remote_grants();
DROP FUNCTION IF EXISTS public.scd_ver_remote_grants();
DROP FUNCTION IF EXISTS public.scd_erase_remote_grants(jsonb, bigint);
DROP FUNCTION IF EXISTS public.scd_row_hash_remote_grants(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_json_remote_grants(jsonb);
DROP FUNCTION IF EXISTS public.scd_merge_remote_link_codes();
DROP FUNCTION IF EXISTS public.scd_tomb_remote_link_codes();
DROP FUNCTION IF EXISTS public.scd_tomb_check_remote_link_codes();
DROP FUNCTION IF EXISTS public.scd_ver_remote_link_codes();
DROP FUNCTION IF EXISTS public.scd_erase_remote_link_codes(jsonb, bigint);
DROP FUNCTION IF EXISTS public.scd_row_hash_remote_link_codes(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_json_remote_link_codes(jsonb);
