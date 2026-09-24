SELECT id
FROM background_jobs
WHERE id = $1 AND lease_id = $2 AND lease_generation = $3 AND generation = $3
  AND lease_expires_at > clock_timestamp()
FOR UPDATE
