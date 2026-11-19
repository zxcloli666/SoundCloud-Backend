DELETE FROM user_followers f
USING (
    SELECT u.target_user_urn FROM user_followers u
    WHERE u.user_id = $1
      AND u.created_at <= $2 AND (u.synced_at IS NULL OR u.synced_at <= $2)
      AND (SELECT s.snapshot_id FROM catalog_collection_seen s
           WHERE s.subject_id = $1 AND s.collection = 'followers' AND s.scope = 'public' AND s.entity_key = u.target_user_urn
      ) IS DISTINCT FROM $3
    LIMIT $4
) gone
WHERE f.user_id = $1 AND f.target_user_urn = gone.target_user_urn
