SELECT deleted_at IS NULL AS "live!"
FROM chat.conversations
WHERE id = $1
FOR KEY SHARE
