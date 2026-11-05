-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS storage_event_state_untranscribed_idx ON storage_event_state (sc_track_id) INCLUDE (uploaded_generation, updated_at) WHERE uploaded_generation > 0 AND transcription_generation IS NULL;
