WITH accepted AS (
    INSERT INTO background_job_enqueues (id)
    VALUES ($1)
    ON CONFLICT (id) DO NOTHING
    RETURNING id
)
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
SELECT accepted.id, $2, $3, $4, $5, $6, $7, $8
FROM accepted
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL
DO UPDATE SET
    payload = EXCLUDED.payload,
    priority = GREATEST(background_jobs.priority, EXCLUDED.priority),
    generation = background_jobs.generation + 1,
    attempts = 0,
    max_attempts = EXCLUDED.max_attempts,
    available_at = LEAST(background_jobs.available_at, EXCLUDED.available_at),
    last_error = NULL,
    updated_at = now()
