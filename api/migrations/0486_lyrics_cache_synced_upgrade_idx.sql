-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS lyrics_cache_synced_upgrade_idx ON lyrics_cache (updated_at, sc_track_id) WHERE source = 'lrclib' AND synced_lrc IS NULL;
