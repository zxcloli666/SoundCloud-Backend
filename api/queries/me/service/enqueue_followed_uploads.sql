INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
SELECT gen_random_uuid(), 'catalog.collection', 'core_bulk',
       'owned-tracks:' || subject_id || ':public',
       jsonb_build_object('version', '1', 'payload', jsonb_build_object(
           'collection', 'owned-tracks', 'subject_id', subject_id, 'owner', false
       )), 5, 8
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
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
