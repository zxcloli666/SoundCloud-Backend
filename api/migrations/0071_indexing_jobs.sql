CREATE INDEX IF NOT EXISTS tracks_indexing_stuck_idx
    ON tracks (index_priority, created_at, sc_track_id)
    WHERE storage_state = 'pending'
       OR (
           index_state = 'pending'
           AND storage_state = 'ok'
           AND s3_verified_at IS NOT NULL
       );

CREATE INDEX IF NOT EXISTS tracks_storage_failed_retry_idx
    ON tracks (updated_at, sc_track_id)
    WHERE storage_state = 'failed';
