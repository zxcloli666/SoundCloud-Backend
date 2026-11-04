UPDATE chat.messages
SET body = NULL
WHERE conversation_id = $1
  AND body IS NOT NULL
  AND (seq = $2 OR (class = 'edit' AND target_seq = $2))
RETURNING seq
