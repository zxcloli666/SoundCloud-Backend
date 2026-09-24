DELETE FROM user_followers f
WHERE f.user_id = $1
  AND f.created_at <= $2 AND (f.synced_at IS NULL OR f.synced_at <= $2)
  AND NOT EXISTS (
      SELECT 1 FROM catalog_collection_seen s
      WHERE s.subject_id = $1 AND s.collection = 'followers' AND s.scope = 'public'
        AND s.snapshot_id = $3 AND s.entity_key = f.target_user_urn
  )
