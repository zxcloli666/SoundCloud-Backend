SELECT state, joined_seq
FROM chat.members
WHERE conversation_id = $1
  AND user_id = $2
