SELECT generation, payload
FROM background_jobs
WHERE dedup_key = 'summary'
