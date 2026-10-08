-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_audio_backfill_idx ON tracks (index_priority, created_at, sc_track_id) WHERE index_state = 'pending' AND storage_state = 'ok' AND s3_verified_at IS NOT NULL AND NOT needs_duration_resolve AND NOT pipeline_held;
