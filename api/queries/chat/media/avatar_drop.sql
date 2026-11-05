DELETE FROM chat.media_refs
WHERE conversation_id = $1
  AND seq = 0
  AND media_id <> ALL($2::uuid[])
RETURNING media_id
