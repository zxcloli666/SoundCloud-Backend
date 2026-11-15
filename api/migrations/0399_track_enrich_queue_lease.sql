SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.track_enrich_queue_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        INSERT INTO public.track_enrich_queue AS queue (
            track_id, enrich_state, index_priority, next_run_at, locked_at, attempts, error
        )
        VALUES (
            NEW.id, NEW.enrich_state, NEW.index_priority, NEW.enrich_next_run_at, NEW.enrich_locked_at,
            NEW.enrich_attempts, NEW.enrich_error
        )
        ON CONFLICT (track_id) DO UPDATE
        SET enrich_state = EXCLUDED.enrich_state,
            index_priority = EXCLUDED.index_priority;
        RETURN NULL;
    END IF;
    INSERT INTO public.track_enrich_queue AS queue (
        track_id, enrich_state, index_priority, next_run_at, locked_at, attempts, error
    )
    VALUES (
        NEW.id, NEW.enrich_state, NEW.index_priority, NEW.enrich_next_run_at, NEW.enrich_locked_at,
        NEW.enrich_attempts, NEW.enrich_error
    )
    ON CONFLICT (track_id) DO UPDATE
    SET enrich_state = EXCLUDED.enrich_state,
        index_priority = EXCLUDED.index_priority,
        next_run_at = CASE
            WHEN OLD.enrich_next_run_at IS DISTINCT FROM NEW.enrich_next_run_at THEN EXCLUDED.next_run_at
            ELSE queue.next_run_at
        END,
        locked_at = CASE
            WHEN OLD.enrich_locked_at IS DISTINCT FROM NEW.enrich_locked_at THEN EXCLUDED.locked_at
            ELSE queue.locked_at
        END,
        attempts = CASE
            WHEN OLD.enrich_attempts IS DISTINCT FROM NEW.enrich_attempts THEN EXCLUDED.attempts
            ELSE queue.attempts
        END,
        error = CASE
            WHEN OLD.enrich_error IS DISTINCT FROM NEW.enrich_error THEN EXCLUDED.error
            ELSE queue.error
        END;
    RETURN NULL;
END
$$;

CREATE OR REPLACE FUNCTION public.star_due_mark_enrich()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM public.star_due_watch AS watch
        WHERE watch.watched_at > now() - interval '1 day'
    ) THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.star_due_marks (sc_track_id)
    SELECT star.sc_track_id
    FROM public.tracks AS track
    JOIN public.star_track_tiers AS star
      ON star.sc_track_id = track.sc_track_id
    WHERE track.id = NEW.track_id;
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF next_run_at, attempts
    ON public.track_enrich_queue
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.next_run_at IS DISTINCT FROM NEW.next_run_at
           OR OLD.attempts IS DISTINCT FROM NEW.attempts))
    EXECUTE FUNCTION public.star_due_mark_enrich();

ALTER TABLE public.track_enrich_queue ENABLE ALWAYS TRIGGER star_due_mark_update;
