SELECT baseline_generation,
       baseline_observation_id,
       projection_revision,
       last_operation_sequence
FROM playlist_membership_state
WHERE playlist_urn = $1
FOR UPDATE
