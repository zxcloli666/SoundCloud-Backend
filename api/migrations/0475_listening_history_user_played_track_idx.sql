-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS listening_history_user_played_track_idx ON public.listening_history (soundcloud_user_id, played_at DESC) INCLUDE (sc_track_id);
