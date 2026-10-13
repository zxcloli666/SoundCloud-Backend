WITH usable AS MATERIALIZED (
    SELECT DISTINCT connection.soundcloud_user_id AS user_id
    FROM soundcloud_connections AS connection
    JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
    WHERE app.active
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
      AND (connection.retry_at IS NULL OR connection.retry_at <= now())
),
due AS MATERIALIZED (
    SELECT waiting.id
    FROM usable
    CROSS JOIN LATERAL (
        SELECT queued.id, queued.next_run_at, queued.created_at
        FROM sync_queue AS queued
        WHERE queued.user_id = usable.user_id
          AND queued.dead = false
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
        LIMIT $2
    ) AS waiting
    ORDER BY waiting.next_run_at, waiting.created_at, waiting.id
    LIMIT $2
),
candidates AS (
    SELECT queued.id
    FROM sync_queue AS queued
    WHERE queued.id = ANY (ARRAY(SELECT id FROM due))
      AND queued.dead = false
      AND queued.next_run_at <= now()
      AND (
          queued.lease_id IS NULL
          OR queued.locked_at < now() - $1::bigint * interval '1 millisecond'
      )
    FOR UPDATE OF queued SKIP LOCKED
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
