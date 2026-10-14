SELECT state.next_run_at AS "waiting_since!"
FROM lyrics_lookup_state AS state
WHERE state.next_run_at <= now()
  AND state.claim_job_id IS NULL
ORDER BY state.next_run_at, state.created_at, state.track_id
LIMIT 1
