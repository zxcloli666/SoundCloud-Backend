SELECT EXISTS (
    SELECT 1 FROM playlists AS playlist
    WHERE playlist.urn = $2 AND playlist.deleted_at IS NULL
      AND (playlist.owner_sc_user_id = $1 OR EXISTS (
          SELECT 1 FROM user_owned_playlists
          WHERE user_id = ANY($3) AND playlist_urn = playlist.urn
      ))
) AS "owns!"
