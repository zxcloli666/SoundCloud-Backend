UPDATE background_jobs
SET lease_expires_at = now() - interval '1 second'
WHERE id = $1
