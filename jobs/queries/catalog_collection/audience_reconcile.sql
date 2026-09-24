DELETE FROM catalog_audience a
WHERE a.subject_urn = $1 AND a.relation = $2
  AND a.created_at <= $3 AND (a.synced_at IS NULL OR a.synced_at <= $3)
  AND NOT EXISTS (
      SELECT 1 FROM catalog_collection_seen s
      WHERE s.subject_id = $4 AND s.collection = $2 AND s.scope = 'public'
        AND s.snapshot_id = $5 AND s.entity_key = a.user_urn
  )
