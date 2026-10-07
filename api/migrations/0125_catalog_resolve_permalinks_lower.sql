CREATE INDEX IF NOT EXISTS tracks_permalink_url_lower_idx ON tracks (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
CREATE INDEX IF NOT EXISTS playlists_permalink_url_lower_idx ON playlists (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
CREATE INDEX IF NOT EXISTS users_permalink_url_lower_idx ON users (lower(permalink_url)) WHERE permalink_url IS NOT NULL;
DROP INDEX IF EXISTS tracks_permalink_url_idx;
DROP INDEX IF EXISTS playlists_permalink_url_idx;
DROP INDEX IF EXISTS users_permalink_url_idx;
