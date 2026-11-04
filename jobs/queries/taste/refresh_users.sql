WITH users AS (
    SELECT DISTINCT regexp_replace(sc_user_id, '^.*:', '') AS user_id
    FROM user_events
    WHERE created_at > $1
      AND created_at <= $2
      AND event_type IN ('like', 'playlist_add', 'full_play', 'skip', 'dislike')
)
SELECT users.user_id AS "user_id!",
       EXISTS (
           SELECT 1
           FROM subscriptions AS subscription
           WHERE subscription.user_urn IN (users.user_id, 'soundcloud:users:' || users.user_id)
             AND subscription.exp_date > extract(epoch FROM now())::bigint
       ) AS "star!"
FROM users
