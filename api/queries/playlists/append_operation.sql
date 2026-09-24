INSERT INTO playlist_membership_operations (
    operation_id,
    playlist_urn,
    sequence,
    actor_sc_user_id,
    idempotency_key,
    request_fingerprint,
    base_baseline_generation,
    base_observation_id,
    expected_projection_revision,
    accepted_projection_revision,
    kind,
    track_id,
    left_anchor_track_id,
    right_anchor_track_id,
    boundary,
    ordered_track_ids
)
VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9::bigint, $9::bigint + 1,
    $10, $11, $12, $13, $14, $15
)
