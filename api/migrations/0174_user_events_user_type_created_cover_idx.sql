-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS user_events_user_type_created_cover_idx ON user_events (sc_user_id, event_type, created_at DESC) INCLUDE (sc_track_id, weight);
