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
  AND NOT (
      lease_id IS NULL
      AND lease_generation IS NULL
      AND locked_at IS NULL
      AND next_run_at <= now()
  )
