-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artist_coplay_b_edges_idx ON public.artist_coplay (b_id, weight DESC) INCLUDE (a_id);
