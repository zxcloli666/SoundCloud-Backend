UPDATE playlist_membership_state AS state
SET reconcile_generation = state.reconcile_generation + 1,
    updated_at = clock_timestamp()
FROM playlists AS playlist
WHERE state.playlist_urn = $1
  AND playlist.urn = state.playlist_urn
  AND playlist.deleted_at IS NULL
RETURNING state.reconcile_generation,
          state.baseline_generation,
          state.last_operation_sequence,
          COALESCE(
              playlist.owner_sc_user_id,
              (
                  SELECT owned.user_id
                  FROM user_owned_playlists AS owned
                  WHERE owned.playlist_urn = state.playlist_urn
                  ORDER BY owned.progress DESC,
                           owned.synced_at DESC NULLS LAST,
                           owned.created_at DESC,
                           owned.user_id
                  LIMIT 1
              )
          ) AS owner_sc_user_id
