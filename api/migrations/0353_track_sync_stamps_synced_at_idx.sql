-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS track_sync_stamps_synced_at_idx ON public.track_sync_stamps (sc_synced_at);
