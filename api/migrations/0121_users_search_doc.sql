-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS users_search_doc_gin ON users USING gin (to_tsvector('simple', coalesce(username_normalized, '') || ' ' || coalesce(full_name, '')));
