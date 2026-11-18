DELETE FROM user_followers f
WHERE f.user_id = $1
  AND f.created_at <= $2 AND (f.synced_at IS NULL OR f.synced_at <= $2)
  AND (
      SELECT s.snapshot_id FROM catalog_collection_seen s
      WHERE s.subject_id = $1 AND s.collection = 'followers' AND s.scope = 'public' AND s.entity_key = f.target_user_urn
  ) IS DISTINCT FROM $3
