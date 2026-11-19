WITH recent_likes AS (SELECT sc_track_id
                      FROM user_likes_tracks
                      WHERE user_id = ANY ($1)
                        AND wanted_state = true
                        AND created_at > NOW() - INTERVAL '180 days'
ORDER BY created_at DESC, ctid DESC
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
    ),
    ranked AS (
SELECT
    p.artist_id, p.sc_track_id, ROW_NUMBER() OVER (
    PARTITION BY p.artist_id
    ORDER BY
    CASE WHEN p.sc_track_id = ANY ($3) THEN 1 ELSE 0 END, COALESCE (c.play_count, 0) DESC
    ) AS rn, co.w
FROM co
    JOIN artist_playable_tracks p
ON p.artist_id = co.co_id AND p.ready_home
    LEFT JOIN sc_track_counters c ON c.sc_track_id = p.sc_track_id
    )
SELECT a.id AS "artist_id!", a.name AS "artist_name!", a.avatar_url, r.sc_track_id AS "sc_track_id!"
FROM ranked r
         JOIN artists a ON a.id = r.artist_id
WHERE r.rn = 1
  AND a.merged_into IS NULL
ORDER BY r.w DESC NULLS LAST, a.id
