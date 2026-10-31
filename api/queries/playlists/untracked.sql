SELECT EXISTS (
    SELECT 1 FROM playlists AS playlist
    WHERE playlist.urn = $1
      AND playlist.deleted_at IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM playlist_membership_state AS state WHERE state.playlist_urn = playlist.urn
      )
) AS "untracked!"
