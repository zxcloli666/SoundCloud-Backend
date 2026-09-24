SELECT f.target_user_urn
FROM (
    SELECT target_user_urn, created_at FROM user_followers
    WHERE user_id = $1
    ORDER BY created_at DESC, target_user_urn DESC OFFSET 0
) f
CROSS JOIN LATERAL (
    SELECT 1 FROM users u WHERE u.urn = f.target_user_urn OFFSET 0
) u
ORDER BY f.created_at DESC, f.target_user_urn DESC
LIMIT $2 OFFSET $3
