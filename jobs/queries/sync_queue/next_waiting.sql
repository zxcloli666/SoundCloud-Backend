WITH usable AS MATERIALIZED (
    SELECT DISTINCT ON (connection.soundcloud_user_id)
           connection.soundcloud_user_id AS user_id,
           connection.oauth_app_id::text AS app,
           EXISTS (
               SELECT 1
               FROM subscriptions AS subscription
               WHERE subscription.user_urn = connection.soundcloud_user_id
                 AND subscription.exp_date > extract(epoch FROM now())::bigint
           ) AS star
    FROM soundcloud_connections AS connection
    JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
    WHERE app.active
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
      AND (connection.retry_at IS NULL OR connection.retry_at <= now())
    ORDER BY connection.soundcloud_user_id,
             EXISTS (
                 SELECT 1
                 FROM sync_queue_action_pauses AS pause
                 WHERE pause.oauth_app_id = connection.oauth_app_id
                   AND pause.reason = 'subscription_required'
             ),
             (connection.expires_at > now()) DESC,
             connection.updated_at DESC
)
SELECT min(head.next_run_at) FILTER (WHERE usable.star) AS star,
       min(head.next_run_at) FILTER (WHERE NOT usable.star) AS regular
FROM usable
CROSS JOIN LATERAL (
    SELECT min(queued.next_run_at) AS next_run_at
    FROM sync_queue AS queued
    WHERE queued.user_id = usable.user_id
      AND queued.dead = false
      AND queued.next_run_at <= now()
      AND (
          queued.lease_id IS NULL
          OR queued.locked_at < now() - $1::bigint * interval '1 millisecond'
      )
      AND NOT ((usable.app || ':' || queued.action_type) = ANY ($2::text[]))
) AS head
