CREATE INDEX IF NOT EXISTS tracks_playable_idx
    ON tracks (id) INCLUDE (sc_track_id)
    WHERE sharing = 'public'
      AND superseded_by IS NULL
      AND storage_state = 'ok'
      AND index_state = 'indexed';
