SELECT l.sc_track_id AS id, (EXTRACT(EPOCH FROM l.created_at) * 1000000)::int8 AS "at!"
FROM unnest($1::text[]) AS u(id)
         CROSS JOIN LATERAL (SELECT sc_track_id, created_at
                             FROM user_likes_tracks
                             WHERE user_id = u.id
                               AND wanted_state = true
                               AND created_at > NOW() - INTERVAL '365 days'
                             ORDER BY created_at DESC, sc_track_id DESC
                             LIMIT $2) l
ORDER BY l.created_at DESC, l.sc_track_id DESC
LIMIT $2
