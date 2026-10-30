WITH queued AS MATERIALIZED (
    SELECT user_id, target_urn
    FROM sync_queue
    WHERE dead = false
      AND action_type IN ('follow_user', 'unfollow_user')
),
stale AS (
    (
        SELECT mirror.user_id,
               CASE WHEN mirror.wanted_state THEN 'follow_user' ELSE 'unfollow_user' END AS action_type,
               mirror.target_user_urn AS target_urn
        FROM user_followings AS mirror
        WHERE mirror.progress = true
          AND mirror.wanted_v = 0
          AND COALESCE(mirror.synced_at, mirror.created_at) < now() - interval '15 minutes'
    )
    UNION ALL
    (
        SELECT mirror.user_id,
               CASE WHEN mirror.wanted_state THEN 'follow_user' ELSE 'unfollow_user' END AS action_type,
               mirror.target_user_urn AS target_urn
        FROM user_followings AS mirror
        WHERE mirror.synced_for_v < mirror.wanted_v
          AND mirror.wanted_v < ((extract(epoch FROM now() - interval '15 minutes') * 1000000)::bigint << 4)
    )
),
missing AS (
    SELECT public.scd_uuid('sync_queue', stale.user_id, stale.action_type, stale.target_urn) AS id,
           stale.user_id,
           stale.action_type,
           stale.target_urn
    FROM stale
    WHERE NOT EXISTS (
        SELECT 1
        FROM queued
        WHERE queued.user_id = stale.user_id
          AND queued.target_urn = stale.target_urn
    )
    LIMIT 500
)
INSERT INTO sync_queue (id, user_id, action_type, target_urn, intent_v, exec_generation)
SELECT missing.id,
       missing.user_id,
       missing.action_type,
       missing.target_urn,
       public.scd_purge_floor('sync_queue', jsonb_build_object('id', missing.id)),
       1
FROM missing
ON CONFLICT (user_id, action_type, target_urn) WHERE action_type <> 'comment' DO UPDATE
SET generation = public.scd_hlc(sync_queue.generation),
    cancelled = false
