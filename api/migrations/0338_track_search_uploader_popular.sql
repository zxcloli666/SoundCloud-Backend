-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS track_search_uploader_popular ON public.track_search (uploader_sc_user_id, play_count_sc DESC NULLS LAST, id DESC);
