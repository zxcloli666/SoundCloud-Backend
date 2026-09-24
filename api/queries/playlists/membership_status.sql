SELECT state.baseline_generation,
       state.projection_revision,
       state.projection_track_count,
       state.last_operation_sequence,
       state.committed_operation_sequence,
       state.sync_status,
       state.conflict_code,
       observation.observed_at AS "observed_at?",
       (
           SELECT count(*)::bigint
           FROM playlist_membership_operations AS operation
           WHERE operation.playlist_urn = state.playlist_urn
             AND operation.resolved_at IS NULL
       ) AS "pending_operations!",
       (
           SELECT count(*)::bigint
           FROM playlist_membership_operations AS operation
           WHERE operation.playlist_urn = state.playlist_urn
             AND operation.outcome = 'conflict'
       ) AS "conflicted_operations!"
FROM playlist_membership_state AS state
LEFT JOIN playlist_remote_observations AS observation
  ON observation.playlist_urn = state.playlist_urn
 AND observation.id = state.latest_observation_id
WHERE state.playlist_urn = $1
