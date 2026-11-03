WITH stale AS (
    SELECT playlist.owner_sc_user_id AS user_id, playlist.urn AS target_urn
    FROM playlists AS playlist
    WHERE playlist.sc_desired <> '{}'::jsonb
      AND NOT playlist.sc_write_confirmed
      AND playlist.deleted_at IS NULL
      AND playlist.sc_mutation_observation < ((extract(epoch FROM now() - interval '15 minutes') * 1000000)::bigint << 4)
      AND playlist.owner_sc_user_id IS NOT NULL
      AND NOT EXISTS (
          SELECT 1
          FROM sync_queue AS queued
          WHERE queued.user_id = playlist.owner_sc_user_id
            AND queued.action_type = 'playlist_update'
            AND queued.target_urn = playlist.urn
      )
    ORDER BY playlist.sc_mutation_observation
    LIMIT 500
),
missing AS (
    SELECT public.scd_uuid('sync_queue', stale.user_id, 'playlist_update', stale.target_urn) AS id,
           stale.user_id,
           stale.target_urn
    FROM stale
)
INSERT INTO sync_queue (id, user_id, action_type, target_urn, intent_v, exec_generation)
SELECT missing.id,
       missing.user_id,
       'playlist_update',
       missing.target_urn,
       public.scd_purge_floor('sync_queue', jsonb_build_object('id', missing.id)),
       1
FROM missing
ON CONFLICT (user_id, action_type, target_urn) WHERE action_type <> 'comment' DO NOTHING
