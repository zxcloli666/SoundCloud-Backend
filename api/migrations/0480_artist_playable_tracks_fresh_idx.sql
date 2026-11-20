-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artist_playable_tracks_fresh_idx ON public.artist_playable_tracks (artist_id, sc_created_at DESC) INCLUDE (sc_track_id) WHERE ready_home;
