-- no-transaction
DO $do$
DECLARE
    after_winner uuid := '00000000-0000-0000-0000-000000000000';
    after_loser uuid := '00000000-0000-0000-0000-000000000000';
    next_winner uuid;
    next_loser uuid;
    tries int := 0;
BEGIN
    IF public.scd_mode() = 'split' AND public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    LOOP
        BEGIN
            PERFORM set_config('lock_timeout', '5s', true);
            PERFORM set_config('scd.writer', 'migration', true);
            WITH batch AS MATERIALIZED (
                SELECT loser.superseded_by, loser.id
                FROM public.tracks AS loser
                WHERE loser.superseded_by IS NOT NULL
                  AND (loser.superseded_by, loser.id) > (after_winner, after_loser)
                ORDER BY loser.superseded_by, loser.id
                LIMIT 1000
            ), locked AS MATERIALIZED (
                SELECT loser.id, winner.canonical_track_id
                FROM batch
                JOIN public.tracks AS loser ON loser.id = batch.id
                JOIN public.tracks AS winner ON winner.id = batch.superseded_by
                WHERE loser.canonical_track_id IS NULL
                  AND winner.canonical_track_id IS NOT NULL
                ORDER BY loser.sc_track_id
                FOR NO KEY UPDATE OF loser
            ), fixed AS (
                UPDATE public.tracks AS loser
                SET canonical_track_id = locked.canonical_track_id
                FROM locked
                WHERE loser.id = locked.id
                  AND loser.canonical_track_id IS NULL
            )
            SELECT batch.superseded_by, batch.id INTO next_winner, next_loser
            FROM batch
            ORDER BY batch.superseded_by DESC, batch.id DESC
            LIMIT 1;
        EXCEPTION WHEN lock_not_available OR deadlock_detected THEN
            tries := tries + 1;
            IF tries >= 20 THEN
                RAISE;
            END IF;
            PERFORM pg_sleep(1);
            CONTINUE;
        END;
        COMMIT;
        tries := 0;
        EXIT WHEN next_winner IS NULL;
        after_winner := next_winner;
        after_loser := next_loser;
    END LOOP;
END
$do$;
