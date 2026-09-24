UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL,
    next_run_at = now()
WHERE dead = false
  AND (
      lease_id IS NULL
      OR locked_at < now() - $1::bigint * interval '1 millisecond'
  )
