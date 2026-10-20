SELECT sender_user,
       class,
       created_at,
       expires_at,
       body IS NULL AS "deleted!"
FROM chat.messages
WHERE conversation_id = $1
  AND seq = $2
