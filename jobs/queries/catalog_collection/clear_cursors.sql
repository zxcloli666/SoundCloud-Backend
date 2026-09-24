DELETE FROM catalog_collection_cursors
WHERE subject_id = $1 AND collection = $2 AND scope = $3
