SELECT user_id
FROM chat.usage
WHERE user_id > $1
ORDER BY user_id
LIMIT $2
