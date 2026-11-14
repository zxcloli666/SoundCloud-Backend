UPDATE catalog_collection_sync
SET snapshot_id = $6,
    started_at = now(),
    next_cursor = NULL,
    page_count = 0,
    item_count = 0,
    complete = false,
    updated_at = now()
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND job_id = $4 AND generation = $5
