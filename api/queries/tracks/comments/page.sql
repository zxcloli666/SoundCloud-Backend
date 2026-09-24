SELECT c.id, c.sc_comment_id, c.user_urn, c.body, c.track_position_ms, c.sc_created_at, c.created_at
FROM (
    SELECT id, sc_comment_id, user_urn, body, track_position_ms, sc_created_at, created_at
    FROM track_comments
    WHERE sc_track_id = $1
    ORDER BY created_at DESC, id DESC OFFSET 0
) c
CROSS JOIN LATERAL (
    SELECT 1 FROM users u WHERE u.urn = c.user_urn OFFSET 0
) u
ORDER BY c.created_at DESC, c.id DESC
LIMIT $2 OFFSET $3
