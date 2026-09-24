INSERT INTO catalog_collection_cursors (subject_id, collection, scope, cursor)
VALUES ($1, $2, $3, $4)
ON CONFLICT DO NOTHING
