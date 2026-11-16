WITH wanted AS (
    SELECT DISTINCT unnest($1::text[]) AS sc_track_id
)
SELECT wanted.sc_track_id AS "sc_track_id!",
       lyrics.synced_lrc,
       COALESCE(comments.at, '{}') AS "comment_ms!",
       COALESCE(replays.at, '{}') AS "replay_ms!"
FROM wanted
LEFT JOIN lyrics_cache AS lyrics ON lyrics.sc_track_id = wanted.sc_track_id
LEFT JOIN LATERAL (
    SELECT array_agg(recent.track_position_ms::integer ORDER BY recent.track_position_ms) AS at
    FROM (
        SELECT comment.track_position_ms
        FROM track_comments AS comment
        WHERE comment.sc_track_id = wanted.sc_track_id
          AND comment.track_position_ms BETWEEN 0 AND 2147483647
        ORDER BY comment.created_at DESC, comment.id DESC
        LIMIT $2
    ) AS recent
) AS comments ON true
LEFT JOIN LATERAL (
    SELECT array_agg(landing.at ORDER BY landing.users DESC, landing.at) AS at
    FROM (
        SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY seek.position_ms) AS at,
               count(DISTINCT seek.sc_user_id) AS users
        FROM track_seeks AS seek
        WHERE seek.sc_track_id = wanted.sc_track_id
          AND seek.source = 'player'
          AND seek.created_at > now() - interval '180 days'
        GROUP BY seek.position_ms / $3
        HAVING count(DISTINCT seek.sc_user_id) >= $4
            OR count(DISTINCT seek.sc_user_id) FILTER (WHERE seek.position_ms < seek.from_ms) >= $5
        ORDER BY users DESC
        LIMIT $2
    ) AS landing
) AS replays ON true
