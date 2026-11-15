WITH RECURSIVE ids AS (
    (SELECT soundcloud_user_id AS user_id
     FROM listening_history
     WHERE soundcloud_user_id > $1
     ORDER BY soundcloud_user_id
     LIMIT 1)
    UNION ALL
    SELECT (SELECT soundcloud_user_id
            FROM listening_history
            WHERE soundcloud_user_id > ids.user_id
            ORDER BY soundcloud_user_id
            LIMIT 1)
    FROM ids
    WHERE ids.user_id IS NOT NULL
),
page AS (
    SELECT user_id,
           regexp_replace(user_id, '^.*:', '') AS bare,
           'soundcloud:users:' || regexp_replace(user_id, '^.*:', '') AS urn
    FROM ids
    WHERE user_id IS NOT NULL
    LIMIT $3
),
latest AS (
    SELECT page.*,
           GREATEST(
               (SELECT played_at FROM listening_history WHERE soundcloud_user_id = page.bare ORDER BY played_at DESC LIMIT 1),
               (SELECT played_at FROM listening_history WHERE soundcloud_user_id = page.urn ORDER BY played_at DESC LIMIT 1)
           ) AS last_play_at
    FROM page
)
SELECT latest.user_id AS "user_id!",
       latest.last_play_at AS "last_play_at!",
       recent.plays AS "plays_28?",
       recent.days AS "active_days_28?",
       LEAST(
           (SELECT played_at FROM listening_history WHERE soundcloud_user_id = latest.bare ORDER BY played_at LIMIT 1),
           (SELECT played_at FROM listening_history WHERE soundcloud_user_id = latest.urn ORDER BY played_at LIMIT 1)
       ) AS "first_play_at?"
FROM latest
LEFT JOIN LATERAL (
    SELECT COUNT(*)::int8 AS plays,
           COUNT(DISTINCT played_at::date)::int8 AS days
    FROM listening_history
    WHERE soundcloud_user_id IN (latest.bare, latest.urn)
      AND played_at >= (now() AT TIME ZONE 'utc') - INTERVAL '28 days'
) AS recent ON latest.last_play_at >= $2
ORDER BY latest.user_id
