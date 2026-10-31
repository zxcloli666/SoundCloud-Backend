WITH due AS MATERIALIZED (
    SELECT state.track_id,
           state.sc_track_id,
           state.generation,
           state.wake_message_id,
           wanted.payload
    FROM unnest($1::text[], $2::jsonb[]) AS wanted(sc_track_id, payload)
    JOIN lyrics_lookup_state AS state
      ON state.sc_track_id = wanted.sc_track_id
    WHERE state.wake_message_id IS NOT NULL
      AND state.wake_generation = state.generation
      AND state.wake_durable_at IS NULL
    ORDER BY state.sc_track_id
    FOR UPDATE OF state
),
accepted AS (
    INSERT INTO background_job_enqueues (id)
    SELECT due.wake_message_id
    FROM due
    ORDER BY due.sc_track_id
    ON CONFLICT (id) DO NOTHING
    RETURNING id
),
enqueued AS (
    INSERT INTO background_jobs (
        id,
        kind,
        lane,
        dedup_key,
        payload,
        priority,
        max_attempts,
        available_at
    )
    SELECT due.wake_message_id, $3, $4, due.sc_track_id, due.payload, $5, $6, $7
    FROM due
    JOIN accepted ON accepted.id = due.wake_message_id
    ORDER BY due.sc_track_id
    ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL
    DO UPDATE SET
        lane = EXCLUDED.lane,
        priority = GREATEST(background_jobs.priority, EXCLUDED.priority),
        updated_at = now()
    WHERE background_jobs.priority < EXCLUDED.priority
       OR background_jobs.lane <> EXCLUDED.lane
)
UPDATE lyrics_lookup_state AS state
SET wake_durable_at = now(),
    updated_at = now()
FROM due
WHERE state.track_id = due.track_id
  AND state.generation = due.generation
  AND state.wake_message_id = due.wake_message_id
  AND state.wake_generation = due.generation
  AND state.wake_durable_at IS NULL
