SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.star_due_mark_lookup()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM public.star_due_watch AS watch
        WHERE watch.watched_at > now() - interval '1 day'
    ) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        INSERT INTO public.star_due_marks (sc_track_id)
        SELECT star.sc_track_id
        FROM public.lyrics_lookup_state AS state
        JOIN public.star_track_tiers AS star
          ON star.sc_track_id = state.sc_track_id
        WHERE state.track_id = NEW.track_id;
        RETURN NULL;
    END IF;
    INSERT INTO public.star_due_marks (sc_track_id)
    SELECT star.sc_track_id
    FROM public.lyrics_lookup_state AS state
    JOIN public.star_track_tiers AS star
      ON star.sc_track_id = state.sc_track_id
    WHERE state.track_id = OLD.track_id;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.track_id IS DISTINCT FROM OLD.track_id THEN
            INSERT INTO public.star_due_marks (sc_track_id)
            SELECT star.sc_track_id
            FROM public.lyrics_lookup_state AS state
            JOIN public.star_track_tiers AS star
              ON star.sc_track_id = state.sc_track_id
            WHERE state.track_id = NEW.track_id;
        END IF;
    END IF;
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id
    ON public.lyrics_lookup_state
    FOR EACH ROW
    WHEN (public.scd_derived() AND OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id)
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON public.lyrics_lookup_state
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF track_id, next_run_at, claim_job_id
    ON public.lyrics_lookup_queue
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.track_id IS DISTINCT FROM NEW.track_id
           OR OLD.next_run_at IS DISTINCT FROM NEW.next_run_at
           OR OLD.claim_job_id IS DISTINCT FROM NEW.claim_job_id))
    EXECUTE FUNCTION public.star_due_mark_lookup();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON public.lyrics_lookup_queue
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.star_due_mark_lookup();

ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_row;

ALTER TABLE public.lyrics_lookup_queue ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE public.lyrics_lookup_queue ENABLE ALWAYS TRIGGER star_due_mark_row;
