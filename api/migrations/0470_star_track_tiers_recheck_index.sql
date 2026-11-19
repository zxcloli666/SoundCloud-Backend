-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS star_track_tiers_recheck_idx ON public.star_track_tiers (recheck_at) WHERE recheck_at IS NOT NULL;
