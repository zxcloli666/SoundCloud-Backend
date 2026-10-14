SELECT state.next_run_at AS "waiting_since!"
FROM lyrics_lookup_state AS state
WHERE state.next_run_at <= now()
  AND state.claim_job_id IS NULL
  AND state.priority BETWEEN $1 AND $2
ORDER BY state.priority, state.next_run_at, state.created_at, state.track_id
LIMIT 1
