DELETE FROM chat.media_refs
WHERE conversation_id = $1
  AND seq = ANY($2::bigint[])
  AND seq > 0
RETURNING media_id
