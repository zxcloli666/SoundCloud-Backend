WITH failed AS (
    DELETE FROM background_jobs
    WHERE id = $1
      AND lease_id = $2
      AND lease_generation = $3
      AND generation = $3
    RETURNING id,
              kind,
              lane,
              dedup_key,
              payload,
              priority,
              generation,
              attempts,
              max_attempts,
              created_at
)
INSERT INTO background_job_failures (
    id,
    kind,
    lane,
    dedup_key,
    payload,
    priority,
    generation,
    attempts,
    max_attempts,
    last_error,
    created_at,
    failed_at
)
SELECT id,
       kind,
       lane,
       dedup_key,
       payload,
       priority,
       generation,
       attempts,
       max_attempts,
       $4,
       created_at,
       now()
FROM failed
ON CONFLICT (id, generation) DO UPDATE SET
    kind = EXCLUDED.kind,
    lane = EXCLUDED.lane,
    dedup_key = EXCLUDED.dedup_key,
    payload = EXCLUDED.payload,
    priority = EXCLUDED.priority,
    attempts = EXCLUDED.attempts,
    max_attempts = EXCLUDED.max_attempts,
    last_error = EXCLUDED.last_error,
    created_at = EXCLUDED.created_at,
    failed_at = EXCLUDED.failed_at
