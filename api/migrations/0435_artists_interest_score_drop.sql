DO $$
DECLARE
    tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.artists IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.artists IN ACCESS EXCLUSIVE MODE;
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

ALTER TABLE public.artists
    DROP COLUMN interest_score;

DO $$
BEGIN
    IF public.scd_role() IN ('core', 'api') THEN
        DELETE FROM public.scd_unpublished_columns
        WHERE tbl = 'artists'
          AND col = 'interest_score';
    END IF;
END
$$;
