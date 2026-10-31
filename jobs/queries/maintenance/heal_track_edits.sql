WITH stale AS (
    SELECT track.uploader_sc_user_id AS user_id, track.urn AS target_urn
    FROM tracks AS track
    WHERE track.sc_desired <> '{}'::jsonb
      AND NOT track.sc_write_confirmed
      AND track.deleted_at IS NULL
      AND track.sc_mutation_observation < ((extract(epoch FROM now() - interval '15 minutes') * 1000000)::bigint << 4)
      AND track.uploader_sc_user_id IS NOT NULL
      AND NOT EXISTS (
          SELECT 1
          FROM sync_queue AS queued
          WHERE queued.user_id = track.uploader_sc_user_id
            AND queued.action_type = 'track_update'
            AND queued.target_urn = track.urn
      )
    ORDER BY track.sc_mutation_observation
    LIMIT 500
),
missing AS (
    SELECT public.scd_uuid('sync_queue', stale.user_id, 'track_update', stale.target_urn) AS id,
           stale.user_id,
           stale.target_urn
    FROM stale
)
INSERT INTO sync_queue (id, user_id, action_type, target_urn, intent_v, exec_generation)
SELECT missing.id,
       missing.user_id,
       'track_update',
       missing.target_urn,
       public.scd_purge_floor('sync_queue', jsonb_build_object('id', missing.id)),
       1
FROM missing
ON CONFLICT (user_id, action_type, target_urn) WHERE action_type <> 'comment' DO NOTHING
