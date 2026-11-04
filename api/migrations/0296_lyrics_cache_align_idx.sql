-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS lyrics_cache_align_idx ON lyrics_cache (sc_track_id) INCLUDE (created_at) WHERE NULLIF(btrim(plain_text), '') IS NOT NULL AND NULLIF(btrim(synced_lrc), '') IS NULL;
