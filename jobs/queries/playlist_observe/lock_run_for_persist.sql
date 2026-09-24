SELECT CASE WHEN playlist.deleted_at IS NOT NULL THEN 'deleted' ELSE run.decision END AS "decision!",
       run.reconcile_generation,
       run.captured_baseline_generation,
       run.captured_through_operation_sequence,
       state.reconcile_generation AS state_reconcile_generation,
       state.baseline_generation,
       state.last_operation_sequence,
       state.committed_operation_sequence,
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
FROM playlist_membership_state AS state
JOIN playlists AS playlist
  ON playlist.urn = state.playlist_urn
JOIN playlist_reconcile_runs AS run
  ON run.playlist_urn = state.playlist_urn
WHERE state.playlist_urn = $1
  AND run.id = $2
  AND run.job_id = $3
  AND run.job_generation = $4
FOR UPDATE OF state, run
