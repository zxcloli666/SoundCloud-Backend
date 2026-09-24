UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL,
    last_error = $4,
    next_run_at = now() + $5::bigint * interval '1 second'
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
