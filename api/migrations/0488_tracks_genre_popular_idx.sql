-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_genre_popular_idx ON public.tracks (lower(genre), play_count_sc DESC NULLS LAST) INCLUDE (sc_track_id, primary_artist_id) WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL AND storage_state = 'ok' AND index_state = 'indexed';
