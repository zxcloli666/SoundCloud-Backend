SELECT oauth_app_id,
       action_type,
       GREATEST(ceil(extract(epoch FROM paused_until - now())), 0)::bigint AS "remaining_seconds!",
       strikes
FROM sync_queue_action_pauses
