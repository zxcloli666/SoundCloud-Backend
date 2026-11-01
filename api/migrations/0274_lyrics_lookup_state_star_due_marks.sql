SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, next_run_at, claim_job_id
    ON public.lyrics_lookup_state
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.next_run_at IS DISTINCT FROM NEW.next_run_at
           OR OLD.claim_job_id IS DISTINCT FROM NEW.claim_job_id))
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON public.lyrics_lookup_state
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_row;
