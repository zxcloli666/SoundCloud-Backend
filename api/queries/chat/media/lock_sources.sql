SELECT conversation.id
FROM chat.conversations AS conversation
WHERE conversation.id IN (
    SELECT ref.conversation_id
    FROM chat.media_refs AS ref
    JOIN chat.members AS member
      ON member.conversation_id = ref.conversation_id
     AND member.user_id = $2
    WHERE ref.media_id = ANY($1::uuid[])
)
ORDER BY conversation.id
FOR KEY SHARE OF conversation
