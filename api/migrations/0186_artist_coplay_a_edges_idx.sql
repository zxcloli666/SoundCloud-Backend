-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artist_coplay_a_edges_idx ON artist_coplay (a_id, weight DESC) INCLUDE (b_id);
