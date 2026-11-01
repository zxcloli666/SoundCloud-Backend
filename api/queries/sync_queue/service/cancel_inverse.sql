UPDATE sync_queue
SET cancelled = true
WHERE user_id = $1
  AND action_type = $2
  AND target_urn = $3
  AND NOT cancelled
  AND lease_id IS NULL
  AND remote_attempted_generation IS NULL
  AND remote_completed_generation IS NULL
