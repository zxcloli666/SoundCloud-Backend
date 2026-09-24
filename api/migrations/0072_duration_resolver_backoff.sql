ALTER TABLE tracks
    ADD COLUMN duration_resolve_attempts smallint NOT NULL DEFAULT 0,
    ADD COLUMN duration_resolve_retry_at timestamptz;

CREATE INDEX IF NOT EXISTS tracks_duration_resolve_due_idx
    ON tracks (COALESCE(duration_resolve_retry_at, sc_synced_at), sc_track_id)
    WHERE needs_duration_resolve = true;

DROP INDEX IF EXISTS tracks_duration_resolve_idx;
