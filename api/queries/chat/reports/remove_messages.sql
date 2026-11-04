UPDATE chat.messages
SET body = NULL
WHERE conversation_id = $1
  AND sender_user = $2
  AND body IS NOT NULL
  AND (seq = ANY ($3) OR (class = 'edit' AND target_seq = ANY ($3)))
RETURNING seq
