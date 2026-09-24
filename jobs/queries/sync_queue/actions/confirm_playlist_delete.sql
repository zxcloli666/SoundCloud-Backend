UPDATE playlists SET sc_write_confirmed = true, updated_at = clock_timestamp()
WHERE urn = $1 AND deleted_at IS NOT NULL
