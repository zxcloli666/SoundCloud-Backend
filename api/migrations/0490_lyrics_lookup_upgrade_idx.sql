-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS lyrics_lookup_upgrade_idx ON lyrics_lookup_state (next_run_at, track_id) WHERE last_outcome = 'upgrade';
