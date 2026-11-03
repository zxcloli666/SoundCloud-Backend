SELECT queued.id,
       queued.cancelled,
       queued.intent_v,
       (
           queued.lease_id IS NULL
           AND queued.remote_attempted_generation IS NULL
           AND queued.remote_completed_generation IS NULL
       ) AS "idle!",
       (
           CASE
               WHEN queued.action_type IN ('like_track', 'unlike_track') THEN coalesce((
                   SELECT mirror.wanted_state
                   FROM user_likes_tracks AS mirror
                   WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                     AND mirror.sc_track_id = regexp_replace(queued.target_urn, '^.*:', '')
               ), false)
               WHEN queued.action_type IN ('like_playlist', 'unlike_playlist') THEN coalesce((
                   SELECT mirror.wanted_state
                   FROM user_likes_playlists AS mirror
                   WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                     AND mirror.playlist_urn = queued.target_urn
               ), false)
               WHEN queued.action_type IN ('follow_user', 'unfollow_user') THEN coalesce((
                   SELECT mirror.wanted_state
                   FROM user_followings AS mirror
                   WHERE mirror.user_id = regexp_replace(queued.user_id, '^.*:', '')
                     AND mirror.target_user_urn = queued.target_urn
               ), false)
           END = (queued.action_type IN ('like_track', 'like_playlist', 'follow_user'))
       ) AS "wanted?"
FROM sync_queue AS queued
WHERE queued.id = $1
FOR UPDATE OF queued
