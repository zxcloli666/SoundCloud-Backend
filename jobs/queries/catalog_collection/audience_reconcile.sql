DELETE FROM catalog_audience a
WHERE a.subject_urn = $1 AND a.relation = $2
  AND a.created_at <= $3 AND (a.synced_at IS NULL OR a.synced_at <= $3)
  AND (
      SELECT s.snapshot_id FROM catalog_collection_seen s
      WHERE s.subject_id = $4 AND s.collection = $2 AND s.scope = 'public' AND s.entity_key = a.user_urn
  ) IS DISTINCT FROM $5
