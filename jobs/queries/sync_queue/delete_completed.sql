DELETE FROM sync_queue
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
  AND remote_completed_generation = $3
