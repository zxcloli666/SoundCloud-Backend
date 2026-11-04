-- no-transaction
DO $do$
BEGIN
    IF public.scd_mode() = 'split' AND public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    CREATE TEMP TABLE enrich_exhausted_ids AS
    SELECT id
    FROM public.tracks
    WHERE enrich_state IN ('pending', 'failed')
      AND enrich_attempts >= 5
      AND enrich_locked_at IS NOT NULL
      AND enrich_locked_at < now() - interval '120 seconds';
    COMMIT;
    LOOP
        WITH batch AS (
            DELETE FROM enrich_exhausted_ids
            WHERE ctid IN (SELECT ctid FROM enrich_exhausted_ids LIMIT 500)
            RETURNING id
        ), locked AS (
            SELECT track.id
            FROM public.tracks AS track
            JOIN batch ON batch.id = track.id
            WHERE track.enrich_state IN ('pending', 'failed')
              AND track.enrich_attempts >= 5
            FOR UPDATE OF track SKIP LOCKED
        )
        UPDATE public.tracks AS track
        SET enrich_state = 'pending',
            enrich_attempts = 0,
            enrich_locked_at = NULL,
            enrich_error = NULL
        FROM locked
        WHERE track.id = locked.id;
        COMMIT;
        EXIT WHEN NOT EXISTS (SELECT 1 FROM enrich_exhausted_ids);
        PERFORM pg_sleep(1);
    END LOOP;
    DROP TABLE enrich_exhausted_ids;
END
$do$;
