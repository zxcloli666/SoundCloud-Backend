-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artist_coplay_a_edges_idx ON public.artist_coplay (a_id, weight DESC) INCLUDE (b_id);
