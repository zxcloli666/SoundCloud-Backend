SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, status
    ON lyrics_embedding_wire_state
    FOR EACH ROW
    WHEN (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
       OR OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON lyrics_embedding_wire_state
    FOR EACH ROW
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE lyrics_embedding_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE lyrics_embedding_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;
