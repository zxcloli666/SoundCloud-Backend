SELECT 'soundcloud:users:' || a.user_id AS "user_urn!"
FROM (
    SELECT user_id, created_at FROM catalog_audience_ids
    WHERE subject_id = $1 AND relation = $2
    ORDER BY created_at DESC, user_id DESC OFFSET 0
) a
CROSS JOIN LATERAL (
    SELECT 1 FROM users u WHERE u.urn = 'soundcloud:users:' || a.user_id OFFSET 0
) u
ORDER BY a.created_at DESC, a.user_id DESC
LIMIT $3 OFFSET $4
