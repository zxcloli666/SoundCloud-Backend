SELECT media_id
FROM chat.media_refs
WHERE conversation_id = $1
  AND seq = 0
