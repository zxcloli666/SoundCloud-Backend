-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS track_search_title_popular ON public.track_search (title_normalized, play_count_sc DESC, id DESC) WHERE play_count_sc >= 100000;
