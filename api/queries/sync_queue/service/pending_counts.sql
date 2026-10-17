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
    SELECT pause.action_type, pause.paused_until
    FROM sync_queue_action_pauses AS pause
    JOIN writer ON writer.oauth_app_id = pause.oauth_app_id
    WHERE pause.paused_until > now() - interval '10 minutes'
),
rows AS (
    SELECT queued.dead,
           queued.dead AND queued.failed_at > now() - interval '30 days' AS recent_dead,
           queued.retry_count,
           queued.action_type IN (
               'like_track', 'unlike_track', 'like_playlist', 'unlike_playlist',
               'follow_user', 'unfollow_user'
           ) AS healed,
           CASE
               WHEN queued.dead THEN NULL
               WHEN hold.action_type IS NOT NULL THEN GREATEST(hold.paused_until, now())
               WHEN queued.retry_count = 0
                    AND queued.last_error IS NOT NULL
                    AND queued.next_run_at > now() THEN queued.next_run_at
           END AS delayed_until
    FROM sync_queue AS queued
    LEFT JOIN holds AS hold ON hold.action_type = queued.action_type
    WHERE queued.user_id = ANY ($1)
      AND NOT (queued.dead AND queued.action_type = 'comment')
)
SELECT count(*) FILTER (
           WHERE NOT dead AND (retry_count = 0 OR delayed_until IS NOT NULL)
       ) AS "pending!",
       count(*) FILTER (
           WHERE dead OR (retry_count > 0 AND delayed_until IS NULL)
       ) AS "failed!",
       count(*) FILTER (WHERE delayed_until IS NOT NULL) AS "delayed!",
       count(*) FILTER (WHERE recent_dead AND NOT healed) AS "dead!",
       ceil(extract(epoch FROM min(delayed_until) - now()))::bigint AS retry_in_sec,
       (
           (
               NOT EXISTS (SELECT 1 FROM writer)
               AND EXISTS (SELECT 1 FROM oauth_apps WHERE active)
           )
           OR (
               EXISTS (SELECT 1 FROM writer WHERE refused)
               AND EXISTS (
                   SELECT 1
                   FROM oauth_apps AS app
                   WHERE app.active
                     AND app.id NOT IN (SELECT oauth_app_id FROM refused)
               )
           )
       ) AS "needs_relogin!"
FROM rows
