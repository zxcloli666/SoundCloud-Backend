SELECT baseline_generation,
       baseline_observation_id,
       projection_revision,
       last_operation_sequence,
       sync_status,
       EXISTS (
           SELECT 1
           FROM playlist_legacy_membership_intents AS intent
           WHERE intent.playlist_urn = state.playlist_urn
             AND intent.classification NOT IN ('resolved', 'abandoned')
       ) AS "has_legacy_intents!"
FROM playlist_membership_state AS state
WHERE playlist_urn = $1
FOR UPDATE
