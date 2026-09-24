UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
