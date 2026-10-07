-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS playlists_permalink_url_lower_idx ON playlists (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
