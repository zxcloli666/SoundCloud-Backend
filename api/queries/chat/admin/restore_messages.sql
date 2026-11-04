UPDATE chat.messages m
SET body = restored.body
FROM unnest($3::int8[], $4::bytea[]) AS restored (seq, body)
WHERE m.conversation_id = $1
  AND m.sender_user = $2
  AND m.seq = restored.seq
  AND m.body IS NULL
  AND (m.expires_at IS NULL OR m.expires_at > now())
  AND NOT EXISTS (
      SELECT 1
      FROM chat.messages deleted
      WHERE deleted.conversation_id = $1
        AND deleted.class = 'system'
        AND (deleted.target_seq = m.seq OR (m.class = 'edit' AND deleted.target_seq = m.target_seq))
  )
RETURNING m.seq
