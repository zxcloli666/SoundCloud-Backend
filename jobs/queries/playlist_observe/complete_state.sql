UPDATE playlist_membership_state
SET baseline_generation = baseline_generation + 1,
    baseline_observation_id = $2,
    latest_observation_id = $2,
    projection_revision = projection_revision + CASE WHEN $3 THEN 1 ELSE 0 END,
    projection_track_count = $4,
    committed_operation_sequence = GREATEST(committed_operation_sequence, $10),
    reconcile_failure_streak = CASE
        WHEN $6 = 'catalog_incomplete' THEN least(reconcile_failure_streak + 1, 12)
        ELSE 0
    END,
    sync_status = $5,
    conflict_code = $6,
    candidate_fingerprint = $7,
    remote_apply_fingerprint = CASE
        WHEN remote_apply_fingerprint IS DISTINCT FROM $11::bytea
         AND remote_applied_at >= clock_timestamp() - interval '1 hour'
        THEN remote_apply_fingerprint
    END,
    remote_apply_generation = CASE
        WHEN remote_apply_fingerprint IS DISTINCT FROM $11::bytea
         AND remote_applied_at >= clock_timestamp() - interval '1 hour'
        THEN remote_apply_generation
    END,
    remote_applied_at = CASE
        WHEN remote_apply_fingerprint IS DISTINCT FROM $11::bytea
         AND remote_applied_at >= clock_timestamp() - interval '1 hour'
        THEN remote_applied_at
    END,
    next_reconcile_at = clock_timestamp() + CASE
        WHEN $5 = 'clean' THEN interval '5 minutes'
        WHEN $6 = 'catalog_incomplete' THEN make_interval(
            secs => least(
                120 * power(2, least(reconcile_failure_streak, 7))::double precision,
                21600
            )
        )
        WHEN candidate_fingerprint IS NOT DISTINCT FROM $7 THEN interval '6 hours'
        WHEN $5 = 'shadow_ready' THEN interval '1 hour'
        WHEN $5 = 'conflict' THEN interval '15 minutes'
        ELSE interval '1 minute'
    END,
    last_error = $8,
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
  AND reconcile_generation = $9
