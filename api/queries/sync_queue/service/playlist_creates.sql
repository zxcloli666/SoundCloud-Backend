WITH refused AS (
    SELECT DISTINCT oauth_app_id
    FROM sync_queue_action_pauses
    WHERE reason = 'subscription_required'
),
writer AS (
    SELECT connection.oauth_app_id,
           connection.oauth_app_id IN (SELECT oauth_app_id FROM refused) AS refused
    FROM soundcloud_connections AS connection
    JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
    WHERE connection.soundcloud_user_id = ANY ($1)
      AND app.active
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
    ORDER BY 2, (connection.expires_at > now()) DESC, connection.updated_at DESC
    LIMIT 1
),
holds AS (
    SELECT DISTINCT CASE
            WHEN pause.action_type IN ('like_track', 'unlike_track', 'like_playlist', 'unlike_playlist') THEN 'likes'
            WHEN pause.action_type IN ('follow_user', 'unfollow_user') THEN 'follows'
            WHEN pause.action_type LIKE 'playlist\_%' THEN 'playlists'
            WHEN pause.action_type = 'comment' THEN 'comments'
            ELSE pause.action_type
        END AS action_group
    FROM sync_queue_action_pauses AS pause
    JOIN writer ON writer.oauth_app_id = pause.oauth_app_id
    WHERE pause.paused_until > now() - interval '10 minutes'
)
SELECT queued.target_urn,
       coalesce(queued.payload -> 'playlist' ->> 'title', '') AS "title!",
       queued.dead,
       (
           NOT queued.dead
           AND (
               hold.action_group IS NOT NULL
               OR (
                   queued.retry_count = 0
                   AND queued.last_error IS NOT NULL
                   AND queued.next_run_at > now()
               )
           )
       ) AS "delayed!"
FROM sync_queue AS queued
LEFT JOIN holds AS hold ON hold.action_group = CASE
            WHEN queued.action_type IN ('like_track', 'unlike_track', 'like_playlist', 'unlike_playlist') THEN 'likes'
            WHEN queued.action_type IN ('follow_user', 'unfollow_user') THEN 'follows'
            WHEN queued.action_type LIKE 'playlist\_%' THEN 'playlists'
            WHEN queued.action_type = 'comment' THEN 'comments'
            ELSE queued.action_type
        END
WHERE queued.user_id = ANY ($1)
  AND queued.action_type = 'playlist_create'
  AND (NOT queued.dead OR queued.failed_at > now() - interval '30 days')
ORDER BY queued.created_at DESC
LIMIT 101
