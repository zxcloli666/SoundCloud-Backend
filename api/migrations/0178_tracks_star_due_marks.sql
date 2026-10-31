SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE TRIGGER star_due_mark_update
    AFTER UPDATE OF sc_track_id, index_state, storage_state, s3_verified_at, needs_duration_resolve, pipeline_held, transcribe_state, enrich_state, enrich_next_run_at, enrich_attempts, hq_upgrade_pending, storage_quality, hq_upgrade_last_at
    ON tracks
    FOR EACH ROW
    WHEN (OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
       OR OLD.index_state IS DISTINCT FROM NEW.index_state
       OR OLD.storage_state IS DISTINCT FROM NEW.storage_state
       OR OLD.s3_verified_at IS DISTINCT FROM NEW.s3_verified_at
       OR OLD.needs_duration_resolve IS DISTINCT FROM NEW.needs_duration_resolve
       OR OLD.pipeline_held IS DISTINCT FROM NEW.pipeline_held
       OR OLD.transcribe_state IS DISTINCT FROM NEW.transcribe_state
       OR OLD.enrich_state IS DISTINCT FROM NEW.enrich_state
       OR OLD.enrich_next_run_at IS DISTINCT FROM NEW.enrich_next_run_at
       OR OLD.enrich_attempts IS DISTINCT FROM NEW.enrich_attempts
       OR OLD.hq_upgrade_pending IS DISTINCT FROM NEW.hq_upgrade_pending
       OR OLD.storage_quality IS DISTINCT FROM NEW.storage_quality
       OR OLD.hq_upgrade_last_at IS DISTINCT FROM NEW.hq_upgrade_last_at)
    EXECUTE FUNCTION public.star_due_mark();

CREATE OR REPLACE TRIGGER star_due_mark_row
    AFTER INSERT OR DELETE ON tracks
    FOR EACH ROW
    EXECUTE FUNCTION public.star_due_mark();

ALTER TABLE tracks ENABLE ALWAYS TRIGGER star_due_mark_update;

ALTER TABLE tracks ENABLE ALWAYS TRIGGER star_due_mark_row;
