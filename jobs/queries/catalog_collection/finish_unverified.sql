UPDATE catalog_collection_sync
SET complete = true,
    next_cursor = NULL,
    synced_at = now(),
    updated_at = now()
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND job_id = $4 AND generation = $5
