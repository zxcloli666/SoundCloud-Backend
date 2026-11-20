-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS user_events_full_play_idx ON user_events (sc_track_id) WHERE event_type = 'full_play';
