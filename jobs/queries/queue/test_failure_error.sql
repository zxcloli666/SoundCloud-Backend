SELECT last_error
FROM background_job_failures
WHERE id = $1
  AND generation = $2
