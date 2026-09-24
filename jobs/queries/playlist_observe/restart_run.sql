UPDATE playlist_reconcile_runs
SET captured_baseline_generation = $2,
    captured_through_operation_sequence = $3,
    observation_id = NULL,
    candidate_fingerprint = NULL,
    decision = 'started',
    reason = NULL,
    started_at = clock_timestamp(),
    completed_at = NULL
WHERE id = $1
  AND decision IN ('started', 'auth_required', 'retry_wait', 'incomplete')
