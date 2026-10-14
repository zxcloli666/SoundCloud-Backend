-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_liked_quality_upgrade_idx ON tracks (storage_quality, hq_upgrade_last_at NULLS FIRST) WHERE hq_upgrade_pending = true AND index_priority = 1;
