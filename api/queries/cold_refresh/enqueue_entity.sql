INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
VALUES ($1, $2, $3, $4, $5, $6, 8)
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO UPDATE
SET priority = EXCLUDED.priority
WHERE background_jobs.priority < EXCLUDED.priority
