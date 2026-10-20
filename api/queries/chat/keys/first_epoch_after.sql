SELECT min(epoch) AS epoch
FROM chat.conversation_keys
WHERE conversation_id = $1
  AND created_at > $2
