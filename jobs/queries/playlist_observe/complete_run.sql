UPDATE playlist_reconcile_runs
SET observation_id = $2,
    candidate_fingerprint = $3,
    decision = $4,
    reason = $5,
    completed_at = clock_timestamp()
WHERE id = $1
  AND decision = 'started'
