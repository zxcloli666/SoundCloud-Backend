SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, uploaded_generation, transcription_generation
    ON storage_event_state
    FOR EACH ROW
    WHEN (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
       OR OLD.uploaded_generation IS DISTINCT FROM NEW.uploaded_generation
       OR OLD.transcription_generation IS DISTINCT FROM NEW.transcription_generation)
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON storage_event_state
    FOR EACH ROW
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE storage_event_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE storage_event_state ENABLE ALWAYS TRIGGER star_due_mark_row;
