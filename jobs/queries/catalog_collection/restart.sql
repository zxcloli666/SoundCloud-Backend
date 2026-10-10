UPDATE catalog_collection_sync
SET snapshot_id = $6,
    next_cursor = $7,
    item_count = 0,
    complete = false,
    synced_at = CASE WHEN verified_at IS NULL THEN NULL ELSE synced_at END,
    updated_at = now()
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND job_id = $4 AND generation = $5
