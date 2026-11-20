DELETE
FROM listening_history_listened
WHERE soundcloud_user_id = ANY ($1)
