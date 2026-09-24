SELECT sc_track_id
FROM playlist_track_projection
WHERE playlist_urn = $1
ORDER BY position
