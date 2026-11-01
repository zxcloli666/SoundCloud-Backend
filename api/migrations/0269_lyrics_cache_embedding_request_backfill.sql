-- no-transaction
DO $do$
DECLARE
    after_id text := '';
    next_id text;
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
                SELECT wire.sc_track_id,
                       wire.request_message_id,
                       wire.lyrics_created_at
                FROM public.lyrics_embedding_wire_state AS wire
                WHERE wire.request_message_id IS NOT NULL
                  AND wire.sc_track_id > after_id
                ORDER BY wire.sc_track_id
                LIMIT 1000
            ), locked AS MATERIALIZED (
                SELECT cache.sc_track_id,
                       batch.request_message_id,
                       batch.lyrics_created_at
                FROM batch
                JOIN public.lyrics_cache AS cache ON cache.sc_track_id = batch.sc_track_id
                WHERE cache.embedding_request_id IS NULL
                ORDER BY cache.sc_track_id
                FOR NO KEY UPDATE OF cache
            ), fixed AS (
                UPDATE public.lyrics_cache AS cache
                SET embedding_request_id = locked.request_message_id,
                    embedding_request_created_at = locked.lyrics_created_at
                FROM locked
                WHERE cache.sc_track_id = locked.sc_track_id
            )
            SELECT batch.sc_track_id INTO next_id
            FROM batch
            ORDER BY batch.sc_track_id DESC
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
        EXIT WHEN next_id IS NULL;
        after_id := next_id;
    END LOOP;
END
$do$;
