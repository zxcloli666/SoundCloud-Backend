UPDATE sync_queue
SET cancelled = true
WHERE user_id = $1 AND target_urn = $2 AND action_type = 'track_update' AND NOT cancelled
