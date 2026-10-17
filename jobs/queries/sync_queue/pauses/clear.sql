DELETE FROM sync_queue_action_pauses
WHERE oauth_app_id = $1
  AND (action_type = $2 OR reason = 'subscription_required')
RETURNING action_type
