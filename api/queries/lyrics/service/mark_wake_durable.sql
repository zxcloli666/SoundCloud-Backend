UPDATE lyrics_lookup_state
SET wake_durable_at = now(),
    updated_at = now()
WHERE track_id = $1
  AND generation = $2
  AND wake_message_id = $3
  AND wake_generation = $2
  AND wake_durable_at IS NULL
