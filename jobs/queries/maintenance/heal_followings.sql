INSERT INTO sync_queue (user_id, action_type, target_urn)
SELECT mirror.user_id,
       CASE WHEN mirror.wanted_state THEN 'follow_user' ELSE 'unfollow_user' END,
       mirror.target_user_urn
FROM user_followings AS mirror
WHERE mirror.progress = true
  AND COALESCE(mirror.synced_at, mirror.created_at) < now() - interval '15 minutes'
  AND NOT EXISTS (
      SELECT 1
      FROM sync_queue AS queued
      WHERE queued.user_id = mirror.user_id
        AND queued.dead = false
        AND queued.target_urn = mirror.target_user_urn
        AND queued.action_type IN ('follow_user', 'unfollow_user')
  )
LIMIT 500
ON CONFLICT (user_id, action_type, target_urn) WHERE action_type <> 'comment' DO UPDATE
SET next_run_at = now(),
    dead = false,
    retry_count = 0,
    last_error = NULL,
    failed_at = NULL,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    generation = sync_queue.generation + 1
