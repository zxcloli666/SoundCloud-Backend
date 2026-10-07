-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS albums_search_doc_gin ON albums USING gin (to_tsvector('simple', coalesce(normalized_title, '')));
