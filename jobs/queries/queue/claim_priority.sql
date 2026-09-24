WITH candidates AS (
    SELECT id
    FROM background_jobs
    WHERE available_at <= now()
      AND attempts < max_attempts
      AND lease_id IS NULL
      AND kind = ANY($2::text[])
      AND lane = $5
    ORDER BY priority DESC, available_at, created_at, id
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
UPDATE background_jobs AS job
SET lease_id = gen_random_uuid(),
    lease_generation = job.generation,
    leased_by = $3,
    lease_expires_at = now() + $4::bigint * interval '1 millisecond',
    attempts = job.attempts + 1,
    updated_at = now()
FROM candidates
WHERE job.id = candidates.id
RETURNING job.id,
          job.kind,
          job.dedup_key,
          job.payload,
          job.lease_generation AS "generation!",
          job.attempts,
          job.max_attempts,
          job.lease_id AS "lease_id!"
