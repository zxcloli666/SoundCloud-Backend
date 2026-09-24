SELECT CASE $1::text
    WHEN 'track' THEN EXISTS(SELECT 1 FROM tracks WHERE sc_track_id = $2 AND deleted_at IS NOT NULL)
    WHEN 'playlist' THEN EXISTS(SELECT 1 FROM playlists WHERE sc_playlist_id = $2 AND deleted_at IS NOT NULL)
    ELSE false
END AS "deleted!"
