UPDATE background_jobs
SET lease_expires_at = now() + $4::bigint * interval '1 millisecond',
    updated_at = now()
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
