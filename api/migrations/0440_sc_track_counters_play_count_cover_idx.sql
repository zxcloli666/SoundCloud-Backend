-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS sc_track_counters_play_count_cover_idx ON public.sc_track_counters (sc_track_id) INCLUDE (play_count);
