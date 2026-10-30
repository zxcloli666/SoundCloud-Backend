WITH locked AS MATERIALIZED (
    SELECT id,
           action_type,
           cancelled,
           lease_id,
           remote_attempted_generation,
           remote_completed_generation
    FROM sync_queue
    WHERE user_id = $1
      AND action_type IN ($2, $3)
      AND target_urn = $4
    ORDER BY id
    FOR UPDATE
)
SELECT coalesce(
           (array_agg(id) FILTER (WHERE action_type = $2))[1],
           public.scd_uuid('sync_queue', $1, $2::text, $4)
       ) < coalesce(
           (array_agg(id) FILTER (WHERE action_type = $3))[1],
           public.scd_uuid('sync_queue', $1, $3::text, $4)
       ) AS "own_first!",
       coalesce(bool_or(
           action_type = $3
           AND NOT cancelled
           AND lease_id IS NULL
           AND remote_attempted_generation IS NULL
           AND remote_completed_generation IS NULL
       ), false) AS "annuls!"
FROM locked
