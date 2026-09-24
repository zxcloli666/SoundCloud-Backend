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
WHERE playlist.urn = $1 AND playlist.deleted_at IS NULL
ON CONFLICT (playlist_urn) DO UPDATE
SET next_reconcile_at = clock_timestamp(),
    updated_at = clock_timestamp()
WHERE playlist_membership_state.last_operation_sequence
          <= playlist_membership_state.committed_operation_sequence
  AND (
      SELECT playlist.track_count
      FROM playlists AS playlist
      WHERE playlist.urn = $1 AND playlist.deleted_at IS NULL
  ) <> playlist_membership_state.projection_track_count
