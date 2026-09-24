INSERT INTO catalog_collection_seen (subject_id, collection, scope, entity_key, snapshot_id)
SELECT $1, $2, $3, key, $4 FROM unnest($5::text[]) AS key
ON CONFLICT (subject_id, collection, scope, entity_key) DO UPDATE
SET snapshot_id = EXCLUDED.snapshot_id
