-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS track_search_doc_gin ON public.track_search USING gin (to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')));
