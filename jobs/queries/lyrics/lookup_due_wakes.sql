SELECT state.sc_track_id AS "sc_track_id!"
FROM lyrics_lookup_queue AS queue
JOIN lyrics_lookup_state AS state
  ON state.track_id = queue.track_id
WHERE queue.wake_at IS NOT NULL
  AND queue.claim_job_id IS NULL
  AND state.wake_message_id IS NOT NULL
  AND state.wake_generation = state.generation
  AND state.wake_durable_at IS NULL
ORDER BY queue.wake_at, queue.track_id
LIMIT $1
