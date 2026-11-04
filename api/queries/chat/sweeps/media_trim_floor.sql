SELECT min(greatest(
           CASE
               WHEN conversation.kind = 'group' AND conversation.settings -> 'history_visible' = 'false'::jsonb
                   THEN member.joined_seq
               ELSE 0
           END,
           member.hidden_before_seq
       )) AS floor
FROM chat.conversations AS conversation
JOIN chat.members AS member
  ON member.conversation_id = conversation.id
WHERE conversation.id = $1
  AND conversation.kind <> 'room'
  AND conversation.deleted_at IS NULL
  AND member.state IN ('active', 'invited')
