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
DO NOTHING
