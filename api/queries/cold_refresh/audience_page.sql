SELECT a.user_urn
FROM (
    SELECT user_urn, created_at FROM catalog_audience
    WHERE subject_urn = $1 AND relation = $2
    ORDER BY created_at DESC, user_urn DESC OFFSET 0
) a
CROSS JOIN LATERAL (
    SELECT 1 FROM users u WHERE u.urn = a.user_urn OFFSET 0
) u
ORDER BY a.created_at DESC, a.user_urn DESC
LIMIT $3 OFFSET $4
