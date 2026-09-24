UPDATE background_jobs
SET lease_id = NULL,
    lease_generation = NULL,
    leased_by = NULL,
    lease_expires_at = NULL,
    attempts = GREATEST(attempts - 1, 0),
    available_at = now() + $4::bigint * interval '1 millisecond',
    last_error = $5,
    updated_at = now()
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
  AND lease_expires_at > now()
