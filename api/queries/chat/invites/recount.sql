UPDATE chat.conversations
SET member_count = (
    SELECT count(*)
    FROM chat.members
    WHERE conversation_id = $1
      AND state IN ('active', 'invited')
)
WHERE id = $1
