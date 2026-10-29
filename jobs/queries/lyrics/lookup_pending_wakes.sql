SELECT sc_track_id AS "sc_track_id!"
FROM lyrics_lookup_state
WHERE sc_track_id = ANY($1::text[])
  AND wake_message_id IS NOT NULL
  AND wake_generation = generation
  AND wake_durable_at IS NULL
