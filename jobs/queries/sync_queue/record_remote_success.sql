UPDATE sync_queue
SET remote_completed_generation = $4,
    remote_result = $5
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $4
  AND remote_attempted_generation = $4
  AND remote_completed_generation IS NULL
