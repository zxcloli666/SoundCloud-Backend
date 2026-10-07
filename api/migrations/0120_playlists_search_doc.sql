-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS playlists_search_doc_gin ON playlists USING gin (to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, ''))) WHERE sharing = 'public' AND deleted_at IS NULL;
