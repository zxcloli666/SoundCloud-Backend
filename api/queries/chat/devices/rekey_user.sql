UPDATE chat.conversations AS conversation
SET rekey = true
FROM chat.members AS member
WHERE member.conversation_id = conversation.id
  AND member.user_id = $1
  AND member.state IN ('active', 'invited')
  AND conversation.deleted_at IS NULL
  AND NOT conversation.rekey
