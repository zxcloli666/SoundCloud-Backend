SELECT payload AS "payload!"
FROM sync_queue
WHERE user_id = $1
  AND action_type = 'track_update'
  AND target_urn = $2
