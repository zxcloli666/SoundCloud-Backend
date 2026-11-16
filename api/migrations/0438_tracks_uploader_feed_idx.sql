-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_uploader_feed_idx ON public.tracks (uploader_sc_user_id, release_date DESC NULLS LAST, sc_created_at DESC NULLS LAST, sc_track_id DESC) INCLUDE (sharing, deleted_at, superseded_by) WHERE uploader_sc_user_id IS NOT NULL;
