SELECT MIN(played_at) AS first_played_at
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
