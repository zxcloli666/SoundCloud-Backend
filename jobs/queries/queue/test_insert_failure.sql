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
    created_at
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
       'previous failure',
       created_at
FROM background_jobs
WHERE id = $1
