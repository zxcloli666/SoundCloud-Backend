DELETE FROM catalog_collection_seen
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND snapshot_id <> $4
