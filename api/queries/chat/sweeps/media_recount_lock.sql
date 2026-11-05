SELECT user_id
FROM chat.usage
WHERE user_id = ANY($1::text[])
ORDER BY user_id
FOR UPDATE
