DO $$
DECLARE
    tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.tracks IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.tracks IN ACCESS EXCLUSIVE MODE;
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

ALTER TABLE public.tracks
    DROP COLUMN work_key,
    DROP COLUMN recording_key,
    DROP COLUMN work_normalizer_version;

DO $$
BEGIN
    IF public.scd_role() IN ('core', 'api') THEN
        DELETE FROM public.scd_unpublished_columns
        WHERE tbl = 'tracks'
          AND col IN ('work_key', 'recording_key', 'work_normalizer_version');
    END IF;
END
$$;
