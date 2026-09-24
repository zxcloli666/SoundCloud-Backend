-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS rec_impressions_user_track_shown_idx ON rec_impressions (sc_user_id, sc_track_id, shown_at DESC);
