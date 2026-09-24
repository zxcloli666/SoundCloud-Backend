UPDATE playlists
SET track_count = $2,
    updated_at = clock_timestamp()
WHERE urn = $1
