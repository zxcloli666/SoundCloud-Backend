CREATE INDEX IF NOT EXISTS tracks_stored_idx
    ON tracks (id) INCLUDE (sc_track_id)
    WHERE sharing = 'public'
      AND superseded_by IS NULL
      AND storage_state = 'ok'
      AND needs_duration_resolve = false;
