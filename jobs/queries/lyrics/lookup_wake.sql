SELECT track_id,
       generation,
       wake_message_id AS "wake_message_id!"
FROM lyrics_lookup_state
WHERE sc_track_id = $1
  AND wake_message_id IS NOT NULL
  AND wake_generation = generation
  AND wake_durable_at IS NULL
FOR UPDATE
