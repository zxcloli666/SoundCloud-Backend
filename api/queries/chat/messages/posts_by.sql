SELECT count(*) AS "count!"
FROM chat.messages
WHERE conversation_id = $1
  AND sender_user = $2
  AND class = 'post'
