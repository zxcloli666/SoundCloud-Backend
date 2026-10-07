-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS users_permalink_url_lower_idx ON users (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
