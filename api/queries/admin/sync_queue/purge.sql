DELETE
FROM sync_queue
WHERE retry_count >= $1
  AND lease_id IS NULL
  AND remote_attempted_generation IS NULL
  AND remote_completed_generation IS NULL
