DELETE FROM background_jobs
WHERE kind = 'indexing.track'
  AND dedup_key = $1
  AND lease_id IS NULL
