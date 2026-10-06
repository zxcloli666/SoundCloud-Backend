-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_search_doc_gin ON tracks USING gin (to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, ''))) WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL;
