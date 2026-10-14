-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artists_account_walk_star_idx ON artists (last_account_walk_at NULLS FIRST) WHERE is_star AND merged_into IS NULL AND has_sc_account;
