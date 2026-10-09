INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
SELECT gen_random_uuid(),
       'catalog.refresh',
       $1,
       'track:' || missing.sc_track_id || ':public',
       jsonb_build_object(
           'version', '1',
           'payload', jsonb_build_object('entity', 'track', 'sc_id', missing.sc_track_id, 'owner_id', NULL)
       ),
       15,
       8
FROM (SELECT DISTINCT sc_track_id FROM unnest($2::text[]) AS candidate(sc_track_id)) AS missing
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO UPDATE
SET priority = EXCLUDED.priority
WHERE background_jobs.priority < EXCLUDED.priority
