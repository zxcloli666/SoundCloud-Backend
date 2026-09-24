SELECT snapshot_id, started_at, next_cursor, page_count, item_count, complete
FROM catalog_collection_sync
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND job_id = $4 AND generation = $5
FOR UPDATE
