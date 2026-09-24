WITH cleared AS (
    DELETE FROM catalog_audience WHERE subject_urn = $1
)
DELETE FROM catalog_collection_sync
WHERE subject_id = $2 AND scope = 'public' AND collection = ANY($3)
