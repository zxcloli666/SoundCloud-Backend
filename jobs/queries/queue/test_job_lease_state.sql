SELECT attempts, lease_id
FROM background_jobs
WHERE id = $1
