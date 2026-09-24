SELECT sc_track_id
FROM playlist_remote_snapshot_tracks
WHERE snapshot_id = $1
ORDER BY position
