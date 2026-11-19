DELETE FROM catalog_collection_seen
WHERE subject_id = $1 AND collection = $2 AND scope = $3
  AND (scope = 'public' OR snapshot_id <> $4)
