SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, plain_text, synced_lrc, embedded_at, embedding_state
    ON public.lyrics_cache
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.plain_text IS DISTINCT FROM NEW.plain_text
           OR OLD.synced_lrc IS DISTINCT FROM NEW.synced_lrc
           OR OLD.embedded_at IS DISTINCT FROM NEW.embedded_at
           OR OLD.embedding_state IS DISTINCT FROM NEW.embedding_state))
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON public.lyrics_cache
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE public.lyrics_cache ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE public.lyrics_cache ENABLE ALWAYS TRIGGER star_due_mark_row;
