DO $$
DECLARE
    tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.lyrics_lookup_state IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.lyrics_lookup_state IN ACCESS EXCLUSIVE MODE;
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

ALTER TABLE public.lyrics_lookup_state
    DROP CONSTRAINT lyrics_lookup_state_claim_valid,
    DROP CONSTRAINT lyrics_lookup_state_generation_valid,
    DROP COLUMN attempts,
    DROP COLUMN last_attempt_at,
    DROP COLUMN claim_job_id,
    DROP COLUMN claim_job_generation,
    DROP COLUMN claim_job_lease_id,
    DROP COLUMN claim_state_generation,
    DROP COLUMN claim_expires_at,
    ADD CONSTRAINT lyrics_lookup_state_generation_valid CHECK (
        generation > 0
        AND algorithm_version > 0
        AND source_set_version > 0
        AND miss_streak >= 0
        AND failure_streak >= 0
    ) NOT VALID,
    SET (fillfactor = 85);

DO $$
BEGIN
    IF public.scd_role() IN ('core', 'api') THEN
        DELETE FROM public.scd_unpublished_columns
        WHERE tbl = 'lyrics_lookup_state'
          AND col IN (
              'attempts', 'last_attempt_at', 'claim_job_id', 'claim_job_generation',
              'claim_job_lease_id', 'claim_state_generation', 'claim_expires_at'
          );
    END IF;
END
$$;
