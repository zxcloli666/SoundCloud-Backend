SELECT synced_at
FROM catalog_collection_sync
WHERE subject_id = $1 AND collection = $2 AND scope = $3
