-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS artists_search_doc_gin ON artists USING gin (to_tsvector('simple', coalesce(normalized_name, ''))) WHERE merged_into IS NULL;
