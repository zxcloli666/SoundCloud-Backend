CREATE INDEX tracks_permalink_url_idx ON tracks (permalink_url) WHERE permalink_url IS NOT NULL;
CREATE INDEX playlists_permalink_url_idx ON playlists (permalink_url) WHERE permalink_url IS NOT NULL;
CREATE INDEX users_permalink_url_idx ON users (permalink_url) WHERE permalink_url IS NOT NULL;
