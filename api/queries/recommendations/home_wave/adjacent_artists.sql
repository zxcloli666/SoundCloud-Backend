WITH recent_likes AS (SELECT l.sc_track_id
               FROM unnest($1::text[]) AS u(id)
                        CROSS JOIN LATERAL (SELECT sc_track_id, created_at
                                            FROM user_likes_tracks
                                            WHERE user_id = u.id
                                              AND wanted_state = true
                                              AND created_at > NOW() - INTERVAL '180 days'
                                            ORDER BY created_at DESC, sc_track_id DESC
                                            LIMIT 300) l
               ORDER BY l.created_at DESC, l.sc_track_id DESC
               LIMIT 300
    ),
    user_artists AS (
SELECT DISTINCT ta.artist_id
FROM recent_likes rl
    JOIN tracks it
ON it.sc_track_id = rl.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
ORDER BY ta.artist_id
    LIMIT 100
    ),
    edges AS (
SELECT e.b_id AS co_id, e.weight
FROM user_artists ua
         CROSS JOIN LATERAL (SELECT b_id, weight
                             FROM artist_coplay
                             WHERE a_id = ua.artist_id
                               AND b_id NOT IN (SELECT artist_id FROM user_artists)
                             ORDER BY weight DESC, b_id
                             LIMIT $2) e
UNION ALL
SELECT e.a_id, e.weight
FROM user_artists ua
         CROSS JOIN LATERAL (SELECT a_id, weight
                             FROM artist_coplay
                             WHERE b_id = ua.artist_id
                               AND a_id NOT IN (SELECT artist_id FROM user_artists)
                             ORDER BY weight DESC, a_id
                             LIMIT $2) e
    ),
    co AS (
SELECT co_id, MAX (weight) AS w
FROM edges
GROUP BY co_id
ORDER BY w DESC, co_id
    LIMIT $2
    )
SELECT a.id AS "artist_id!", a.name AS "artist_name!", a.avatar_url
FROM co
         JOIN artists a ON a.id = co.co_id
WHERE a.merged_into IS NULL
ORDER BY co.w DESC, a.id
