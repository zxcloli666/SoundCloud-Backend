SELECT LEAST(
    (SELECT first_seen_at FROM chat.users WHERE user_id = $1),
    (SELECT min(created_at) FROM chat.devices WHERE user_id = $1)
) AS first_seen
