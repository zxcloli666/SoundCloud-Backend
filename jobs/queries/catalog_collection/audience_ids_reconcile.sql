DELETE FROM catalog_audience_ids a
WHERE a.subject_id = $1 AND a.relation = $2 AND a.created_at <= $3
  AND (
      SELECT s.snapshot_id FROM catalog_collection_seen s
      WHERE s.subject_id = $4 AND s.collection = $5 AND s.scope = 'public'
        AND s.entity_key = 'soundcloud:users:' || a.user_id
  ) IS DISTINCT FROM $6
