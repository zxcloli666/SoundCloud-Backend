-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_permalink_url_lower_idx ON tracks (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
