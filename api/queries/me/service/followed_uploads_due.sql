SELECT subjects.subject_id AS "subject_id!"
FROM (SELECT DISTINCT unnest($1::text[]) AS subject_id) subjects
WHERE subject_id ~ '^[1-9][0-9]*$'
  AND (
      SELECT wanted_state FROM user_followings f
      WHERE f.user_id = ANY(ARRAY[$2, 'soundcloud:users:' || $2])
        AND f.target_user_urn = 'soundcloud:users:' || subjects.subject_id
      ORDER BY (f.user_id = $2) DESC LIMIT 1
  ) IS TRUE
  AND NOT EXISTS (
      SELECT 1 FROM catalog_collection_sync s
      WHERE s.subject_id = subjects.subject_id AND s.collection = 'owned-tracks'
        AND s.scope = 'public' AND s.synced_at BETWEEN now() - interval '5 minutes' AND now()
  )
  AND NOT EXISTS (
      SELECT 1 FROM background_job_failures f
      WHERE f.kind = 'catalog.collection'
        AND f.dedup_key = 'owned-tracks:' || subjects.subject_id || ':public'
        AND f.failed_at > now() - interval '6 hours'
        AND split_part(f.last_error, ':', 1) IN ('not_found', 'forbidden')
  )
