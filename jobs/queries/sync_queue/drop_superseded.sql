WITH superseded AS MATERIALIZED (
    SELECT queued.id, queued.intent_v
    FROM sync_queue AS queued
    WHERE queued.id = $1
      AND queued.lease_id = $2
      AND queued.lease_generation = $3
      AND queued.generation = $4
      AND queued.remote_attempted_generation IS DISTINCT FROM queued.generation
      AND queued.remote_completed_generation IS NULL
      AND CASE
          WHEN queued.action_type IN ('like_track', 'unlike_track') THEN coalesce((
              SELECT mirror.wanted_state <> (queued.action_type = 'like_track')
              FROM user_likes_tracks AS mirror
              WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                AND mirror.sc_track_id = regexp_replace(queued.target_urn, '^.*:', '')
          ), queued.action_type = 'like_track' AND EXISTS (
              SELECT 1
              FROM repl_purges AS receipt
              WHERE receipt.tbl = 'user_likes_tracks'
                AND receipt.pk = jsonb_build_object(
                    'user_id', regexp_replace(queued.user_id, '^.*:', ''),
                    'sc_track_id', regexp_replace(queued.target_urn, '^.*:', '')
                )
                AND receipt.purged_v >= queued.generation
          ))
          WHEN queued.action_type IN ('like_playlist', 'unlike_playlist') THEN coalesce((
              SELECT mirror.wanted_state <> (queued.action_type = 'like_playlist')
              FROM user_likes_playlists AS mirror
              WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                AND mirror.playlist_urn = queued.target_urn
          ), queued.action_type = 'like_playlist' AND EXISTS (
              SELECT 1
              FROM repl_purges AS receipt
              WHERE receipt.tbl = 'user_likes_playlists'
                AND receipt.pk = jsonb_build_object(
                    'user_id', regexp_replace(queued.user_id, '^.*:', ''),
                    'playlist_urn', queued.target_urn
                )
                AND receipt.purged_v >= queued.generation
          ))
          WHEN queued.action_type IN ('follow_user', 'unfollow_user') THEN coalesce((
              SELECT mirror.wanted_state <> (queued.action_type = 'follow_user')
              FROM user_followings AS mirror
              WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                AND mirror.target_user_urn = queued.target_urn
          ), queued.action_type = 'follow_user' AND EXISTS (
              SELECT 1
              FROM repl_purges AS receipt
              WHERE receipt.tbl = 'user_followings'
                AND receipt.pk = jsonb_build_object(
                    'user_id', regexp_replace(queued.user_id, '^.*:', ''),
                    'target_user_urn', queued.target_urn
                )
                AND receipt.purged_v >= queued.generation
          ))
          ELSE false
      END
    FOR UPDATE OF queued
)
SELECT count(*) > 0 AS "dropped!"
FROM superseded
WHERE public.scd_purge_intent('sync_queue', jsonb_build_object('id', superseded.id), superseded.intent_v)
