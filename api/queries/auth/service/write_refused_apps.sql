SELECT DISTINCT oauth_app_id
FROM sync_queue_action_pauses
WHERE reason = 'subscription_required'
