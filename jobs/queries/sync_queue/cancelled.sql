SELECT queued.id
FROM sync_queue AS queued
WHERE queued.cancelled
  AND queued.lease_id IS NULL
  AND queued.remote_attempted_generation IS NULL
  AND queued.remote_completed_generation IS NULL
ORDER BY queued.id
LIMIT $1
