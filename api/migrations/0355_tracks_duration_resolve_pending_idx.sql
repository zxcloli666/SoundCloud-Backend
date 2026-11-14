-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_duration_resolve_pending_idx ON public.tracks (duration_resolve_retry_at, sc_track_id) WHERE needs_duration_resolve = true;
