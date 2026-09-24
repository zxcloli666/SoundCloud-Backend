DELETE FROM background_jobs
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
