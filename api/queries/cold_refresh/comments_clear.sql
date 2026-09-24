WITH cleared AS (
    DELETE FROM track_comments WHERE sc_track_id = $1
)
DELETE FROM catalog_collection_sync
WHERE subject_id = $1 AND scope = 'public' AND collection = 'track-comments'
