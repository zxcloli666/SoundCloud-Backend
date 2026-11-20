-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_album_rollup_idx ON tracks (id) INCLUDE (sc_track_id, duration_ms, release_date);
