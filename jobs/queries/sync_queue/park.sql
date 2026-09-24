UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL,
    dead = true,
    failed_at = now(),
    retry_count = $4,
    last_error = $5,
    next_run_at = 'infinity'
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
