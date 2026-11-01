SELECT sc_track_id AS "sc_track_id!"
FROM lyrics_lookup_state
WHERE wake_message_id IS NOT NULL
  AND wake_generation = generation
  AND wake_durable_at IS NULL
  AND claim_job_id IS NULL
ORDER BY updated_at, track_id
LIMIT $1
