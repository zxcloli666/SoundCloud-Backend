SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, status, quarantine_reason, upload_generation
    ON transcription_wire_state
    FOR EACH ROW
    WHEN (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
       OR OLD.status IS DISTINCT FROM NEW.status
       OR OLD.quarantine_reason IS DISTINCT FROM NEW.quarantine_reason
       OR OLD.upload_generation IS DISTINCT FROM NEW.upload_generation)
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON transcription_wire_state
    FOR EACH ROW
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE transcription_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE transcription_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;
