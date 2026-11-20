-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_primary_artist_plays_idx ON tracks (primary_artist_id) INCLUDE (sc_track_id) WHERE primary_artist_id IS NOT NULL;
