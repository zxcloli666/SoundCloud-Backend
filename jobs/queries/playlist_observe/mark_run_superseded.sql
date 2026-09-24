UPDATE playlist_reconcile_runs
SET observation_id = COALESCE($2, observation_id),
    decision = 'superseded',
    reason = $3,
    completed_at = clock_timestamp()
WHERE id = $1
  AND decision = 'started'
