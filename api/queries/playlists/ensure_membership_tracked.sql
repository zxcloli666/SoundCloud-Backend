INSERT INTO playlist_membership_state (
    playlist_urn,
    projection_track_count,
    sync_status,
    next_reconcile_at
)
SELECT playlist.urn,
       (
           SELECT count(*)::integer
           FROM playlist_track_projection AS projection
           WHERE projection.playlist_urn = playlist.urn
       ),
       'unhydrated',
       clock_timestamp()
FROM playlists AS playlist
WHERE playlist.urn = $1
  AND playlist.deleted_at IS NULL
  AND NOT EXISTS (
      SELECT 1 FROM playlist_membership_state AS state WHERE state.playlist_urn = $1
  )
ON CONFLICT (playlist_urn) DO NOTHING
