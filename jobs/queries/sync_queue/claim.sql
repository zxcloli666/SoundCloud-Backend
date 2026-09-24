WITH candidates AS MATERIALIZED (
    SELECT queued.id
    FROM sync_queue AS queued
    WHERE queued.dead = false
      AND queued.next_run_at <= now()
      AND (
          queued.lease_id IS NULL
          OR queued.locked_at < now() - $1::bigint * interval '1 millisecond'
      )
      AND NOT EXISTS (
          SELECT 1
          FROM sync_queue AS earlier
          WHERE earlier.dead = false
            AND earlier.user_id = queued.user_id
            AND earlier.target_urn = queued.target_urn
            AND (earlier.created_at, earlier.id) < (queued.created_at, queued.id)
      )
    ORDER BY queued.next_run_at, queued.created_at, queued.id
    FOR UPDATE OF queued SKIP LOCKED
    LIMIT $2
)
UPDATE sync_queue AS queued
SET lease_id = gen_random_uuid(),
    lease_generation = queued.generation,
    locked_at = now()
FROM candidates
WHERE queued.id = candidates.id
RETURNING queued.id,
          queued.user_id,
          queued.action_type,
          queued.target_urn,
          queued.payload,
          queued.retry_count,
          queued.generation,
          queued.lease_id AS "lease_id!",
          queued.lease_generation AS "lease_generation!",
          queued.remote_attempted_generation,
          queued.remote_completed_generation,
          queued.remote_result
