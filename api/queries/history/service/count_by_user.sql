SELECT COALESCE(SUM(counted.total), 0)::bigint AS "count!"
FROM (SELECT DISTINCT user_id FROM unnest($1::text[]) AS user_id) AS users
CROSS JOIN LATERAL (
    SELECT COUNT(*) AS total
    FROM listening_history
    WHERE soundcloud_user_id = users.user_id
) AS counted
