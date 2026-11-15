SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description', 'track_sync_stamps', 'track_work_keys', 'track_enrich_queue')
    );

CREATE OR REPLACE FUNCTION public.track_enrich_queue_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
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
        next_run_at = EXCLUDED.next_run_at,
        locked_at = EXCLUDED.locked_at,
        attempts = EXCLUDED.attempts,
        error = EXCLUDED.error
    WHERE (queue.enrich_state, queue.index_priority, queue.next_run_at, queue.locked_at, queue.attempts, queue.error)
          IS DISTINCT FROM
          (EXCLUDED.enrich_state, EXCLUDED.index_priority, EXCLUDED.next_run_at, EXCLUDED.locked_at,
           EXCLUDED.attempts, EXCLUDED.error);
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER track_enrich_queue_update
    AFTER UPDATE OF enrich_state, index_priority, enrich_next_run_at, enrich_locked_at, enrich_attempts, enrich_error
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.enrich_state IS DISTINCT FROM NEW.enrich_state
           OR OLD.index_priority IS DISTINCT FROM NEW.index_priority
           OR OLD.enrich_next_run_at IS DISTINCT FROM NEW.enrich_next_run_at
           OR OLD.enrich_locked_at IS DISTINCT FROM NEW.enrich_locked_at
           OR OLD.enrich_attempts IS DISTINCT FROM NEW.enrich_attempts
           OR OLD.enrich_error IS DISTINCT FROM NEW.enrich_error))
    EXECUTE FUNCTION public.track_enrich_queue_sync();

CREATE OR REPLACE TRIGGER track_enrich_queue_row
    AFTER INSERT ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.track_enrich_queue_sync();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_enrich_queue_update;
ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_enrich_queue_row;
