SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, status, upload_generation, dispatched_at
    ON public.audio_index_wire_state
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.status IS DISTINCT FROM NEW.status
           OR OLD.upload_generation IS DISTINCT FROM NEW.upload_generation
           OR OLD.dispatched_at IS DISTINCT FROM NEW.dispatched_at))
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON public.audio_index_wire_state
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE public.audio_index_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE public.audio_index_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;
