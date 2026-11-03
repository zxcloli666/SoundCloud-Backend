-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artists_mb_only_claim_idx ON artists (mb_next_run_at) WHERE merged_into IS NULL AND NOT crawl_dead AND mb_artist_id IS NOT NULL AND genius_artist_id IS NULL;
