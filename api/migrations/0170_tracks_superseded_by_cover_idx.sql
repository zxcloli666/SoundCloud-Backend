-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_superseded_by_cover_idx ON tracks (superseded_by) INCLUDE (id, duration_ms) WHERE superseded_by IS NOT NULL;
