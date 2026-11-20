WITH recent_likes AS (SELECT l.sc_track_id
               FROM unnest($1::text[]) AS u(id)
                        CROSS JOIN LATERAL (SELECT sc_track_id, created_at
                                            FROM user_likes_tracks
                                            WHERE user_id = u.id
                                              AND wanted_state = true
                                              AND created_at > NOW() - INTERVAL '120 days'
                                            ORDER BY created_at DESC, sc_track_id DESC
                                            LIMIT 200) l
               ORDER BY l.created_at DESC, l.sc_track_id DESC
               LIMIT 200
    ),
    user_artists AS (
SELECT ta.artist_id
FROM recent_likes rl
    JOIN tracks it
ON it.sc_track_id = rl.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
GROUP BY ta.artist_id
HAVING COUNT (*) >= 2
    )
SELECT fresh.sc_track_id AS "sc_track_id!"
FROM user_artists ua
         CROSS JOIN LATERAL (SELECT p.sc_track_id, p.sc_created_at
                             FROM artist_playable_tracks p
                             WHERE p.artist_id = ua.artist_id
                               AND p.ready_home
                               AND p.sc_created_at > NOW() - INTERVAL '90 days'
                               AND NOT (p.sc_track_id = ANY ($2))
                             ORDER BY p.sc_created_at DESC
                             LIMIT $3) fresh
GROUP BY fresh.sc_track_id
ORDER BY MAX(fresh.sc_created_at) DESC, fresh.sc_track_id DESC
    LIMIT $3
