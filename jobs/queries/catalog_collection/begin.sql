INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (subject_id, collection, scope) DO UPDATE SET
    job_id = EXCLUDED.job_id,
    generation = EXCLUDED.generation,
    snapshot_id = EXCLUDED.snapshot_id,
    started_at = now(),
    updated_at = now(),
    next_cursor = NULL,
    page_count = 0,
    item_count = 0,
    complete = false
WHERE catalog_collection_sync.job_id <> EXCLUDED.job_id
   OR catalog_collection_sync.generation <> EXCLUDED.generation
