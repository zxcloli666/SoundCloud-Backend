UPDATE background_jobs
SET attempts = 0
WHERE id = $1 AND lease_id = $2 AND lease_generation = $3 AND generation = $3
  AND lease_expires_at > now()
